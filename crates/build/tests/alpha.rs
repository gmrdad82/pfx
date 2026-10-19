mod common;

use pfx_build::{build, library, write};
use pfx_recipe::parse;
use serde_json::{Value, json};
use std::path::Path;

fn png() -> Vec<u8> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, 2, 2);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    writer
        .write_image_data(&[
            40, 160, 40, 255, 40, 160, 40, 0, 40, 160, 40, 0, 40, 160, 40, 255,
        ])
        .unwrap();
    writer.finish().unwrap();
    out
}

fn pad(data: &mut Vec<u8>) {
    while !data.len().is_multiple_of(4) {
        data.push(0);
    }
}

fn leaf_card(textured: bool) -> Vec<u8> {
    let mut data = Vec::new();
    let mut views = Vec::new();
    let mut put = |data: &mut Vec<u8>, bytes: &[u8], target: Option<u32>| {
        pad(data);
        let mut view = json!({ "buffer": 0, "byteOffset": data.len(), "byteLength": bytes.len() });
        if let Some(target) = target {
            view["target"] = json!(target);
        }
        data.extend_from_slice(bytes);
        views.push(view);
        views.len() - 1
    };
    let floats = |v: &[f32]| v.iter().flat_map(|f| f.to_le_bytes()).collect::<Vec<u8>>();
    let positions = [
        -0.1, 0.0, 0.0, 0.1, 0.0, 0.0, 0.1, 0.2, 0.0, -0.1, 0.2, 0.0_f32,
    ];
    let p = put(&mut data, &floats(&positions), Some(34962));
    let n = put(&mut data, &floats(&[0.0, 0.0, 1.0].repeat(4)), Some(34962));
    let t = put(
        &mut data,
        &floats(&[0.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0]),
        Some(34962),
    );
    let indices: Vec<u8> = [0u16, 1, 2, 0, 2, 3]
        .iter()
        .flat_map(|i| i.to_le_bytes())
        .collect();
    let i = put(&mut data, &indices, Some(34963));
    let image = put(&mut data, &png(), None);
    pad(&mut data);
    let mut material = json!({
        "name": "leaf",
        "alphaMode": "MASK",
        "alphaCutoff": 0.5,
        "doubleSided": false,
        "pbrMetallicRoughness": {
            "baseColorFactor": [0.3, 0.6, 0.2, 1.0],
            "metallicFactor": 0.0,
            "roughnessFactor": 0.8,
        },
    });
    if textured {
        material["pbrMetallicRoughness"]["baseColorTexture"] = json!({ "index": 0 });
    }
    let mut json = json!({
        "asset": { "version": "2.0" },
        "scene": 0,
        "scenes": [{ "nodes": [0] }],
        "nodes": [{ "name": "card", "mesh": 0 }],
        "meshes": [{
            "name": "card",
            "primitives": [{
                "attributes": { "POSITION": 0, "NORMAL": 1, "TEXCOORD_0": 2 },
                "indices": 3,
                "material": 0,
            }],
        }],
        "materials": [material],
        "accessors": [
            { "bufferView": p, "componentType": 5126, "count": 4, "type": "VEC3",
              "min": [-0.1, 0.0, 0.0], "max": [0.1, 0.2, 0.0] },
            { "bufferView": n, "componentType": 5126, "count": 4, "type": "VEC3" },
            { "bufferView": t, "componentType": 5126, "count": 4, "type": "VEC2" },
            { "bufferView": i, "componentType": 5123, "count": 6, "type": "SCALAR" },
        ],
        "bufferViews": views,
        "buffers": [{ "byteLength": data.len() }],
    });
    if textured {
        json["textures"] = json!([{ "source": 0 }]);
        json["images"] = json!([{ "mimeType": "image/png", "bufferView": image }]);
    }
    library::join_glb(&json, &data)
}

fn leaf(glb: &[u8]) -> Value {
    let (json, _) = library::split_glb(glb).unwrap();
    json["materials"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["name"] == "leaf")
        .cloned()
        .unwrap_or_else(|| panic!("no leaf material in {}", json["materials"]))
}

fn assert_mask(material: &Value, what: &str) {
    assert_eq!(material["alphaMode"], "MASK", "{what}: {material}");
    assert!(
        (material["alphaCutoff"].as_f64().unwrap_or(-1.0) - 0.5).abs() < 1e-6,
        "{what}: {material}"
    );
    assert_eq!(material["doubleSided"], json!(false), "{what}: {material}");
}

fn recipe(materials: &str) -> pfx_recipe::Recipe {
    parse(&format!(
        "recipe = 1\nname = \"leaves\"\n\n[output]\nglb = \"leaves.glb\"\nscene = \"leaves.scene.toml\"\n\n[[object]]\nname = \"card\"\nkind = \"mesh\"\npath = \"card.glb\"\n{materials}"
    ))
    .unwrap()
}

fn scene_text(dir: &Path, written: &[std::path::PathBuf]) -> String {
    let mut text = String::new();
    for path in written {
        if path.extension().is_some_and(|e| e == "toml") {
            text.push_str(&std::fs::read_to_string(dir.join(path)).unwrap());
        }
    }
    text
}

#[test]
fn an_imported_mask_leaf_card_stays_a_single_sided_cutout_through_the_build() {
    for textured in [true, false] {
        for materials in ["", "\n[materials.leaf]\nbase = [0.3, 0.6, 0.2]\n"] {
            let what = format!("textured {textured}, recipe material {materials:?}");
            let dir = common::scratch(&format!("alpha-{textured}-{}", materials.len()));
            std::fs::write(dir.join("card.glb"), leaf_card(textured)).unwrap();
            let r = recipe(materials);
            let b = build(&r, &dir).unwrap();
            assert_mask(&leaf(&b.try_glb().unwrap()), &what);
            write(&b, &r, &dir, None).unwrap();
            assert_mask(
                &leaf(&std::fs::read(dir.join("leaves.glb")).unwrap()),
                &what,
            );
        }
    }
}

#[test]
fn a_timeline_scene_keeps_the_cutoff_of_its_cards_and_its_mask_materials() {
    let dir = common::scratch("alpha-timeline");
    std::fs::write(dir.join("card.glb"), leaf_card(false)).unwrap();
    std::fs::write(dir.join("leaf.png"), png()).unwrap();
    let r = parse(
        "recipe = 1\nname = \"leaves\"\n\n[output]\nglb = \"leaves.glb\"\nscene = \"leaves.scene.toml\"\n\n[timeline]\nfps = 10\nframes = 10\n\n[[object]]\nname = \"card\"\nkind = \"mesh\"\npath = \"card.glb\"\n\n[[object]]\nname = \"sprig\"\nkind = \"card\"\nsize = [0.2, 0.3]\nimage = \"leaf.png\"\nalpha_cutoff = 0.4\nkeys = { at = [[0.0, [0.0, 0.0, 0.0]], [1.0, [0.0, 0.5, 0.0]]] }\n",
    )
    .unwrap();
    let b = build(&r, &dir).unwrap();
    assert!(!b.tracks.is_empty());
    let written = write(&b, &r, &dir, None).unwrap();
    let text = scene_text(&dir, &written);
    let scene: toml::Table =
        toml::from_str(&std::fs::read_to_string(dir.join("leaves.scene.toml")).unwrap()).unwrap();
    let objects = scene["object"].as_array().unwrap();
    assert_eq!(objects.len(), 1, "{text}");
    let cutoff = objects[0]
        .get("alpha_cutoff")
        .and_then(toml::Value::as_float);
    assert!(cutoff.is_some_and(|c| (c - 0.4).abs() < 1e-6), "{text}");
    let glb = std::fs::read(dir.join("leaves.glb")).unwrap();
    let material = leaf(&glb);
    let in_library = text.contains("[materials.leaf]") && text.contains("alpha_mode = \"mask\"");
    assert!(
        in_library || material["alphaMode"] == "MASK",
        "{text}\n{material}"
    );
    assert_mask(&material, "the timeline GLB");
}
