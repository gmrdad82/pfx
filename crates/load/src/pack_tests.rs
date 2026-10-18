use std::path::Path;

use serde_json::Value;

use crate::pack::{from_obj, gltf, roughness};
use crate::{Mesh, Source};

fn data() -> Source {
    Source::folder(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data"))
}

fn json(glb: &[u8]) -> Value {
    let glb = gltf::binary::Glb::from_slice(glb).unwrap();
    serde_json::from_slice(&glb.json).unwrap()
}

fn uris(doc: &Value) -> Vec<String> {
    ["buffers", "images"]
        .iter()
        .flat_map(|list| {
            doc.get(list)
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
        })
        .filter_map(|item| item.get("uri").and_then(Value::as_str).map(str::to_string))
        .collect()
}

const PNG: &[u8] = include_bytes!("../tests/data/swatch8.png");

#[test]
fn a_split_gltf_packs_into_one_glb_that_loads_like_the_binary_one() {
    let folder = data();
    let text = folder.read("triangle.gltf").unwrap();
    let packed = gltf(&text, |path| folder.read(path)).unwrap();
    assert!(uris(&json(&packed)).is_empty());
    let glb = Mesh::parse(include_bytes!("../tests/data/triangle.glb")).unwrap();
    assert_eq!(Mesh::parse(&packed).unwrap(), glb);
    let binary = include_bytes!("../tests/data/triangle.glb");
    assert_eq!(gltf(binary, |_| Err("no reads".into())).unwrap(), binary);
}

#[test]
fn external_images_embed_with_their_type_and_reach_the_material() {
    let text = String::from_utf8(data().read("triangle.gltf").unwrap())
        .unwrap()
        .replace(
            "\"materials\":[{\"name\":\"ink\"}]",
            "\"materials\":[{\"name\":\"ink\",\"pbrMetallicRoughness\":{\"baseColorTexture\":{\"index\":0}}}],\
             \"textures\":[{\"source\":0}],\"images\":[{\"uri\":\"textures/paint%20diff.png\"}]",
        );
    let read = |path: &str| match path {
        "textures/paint diff.png" => Ok(PNG.to_vec()),
        other => data().read(other),
    };
    let packed = gltf(text.as_bytes(), read).unwrap();
    let doc = json(&packed);
    assert!(uris(&doc).is_empty());
    assert_eq!(doc["images"][0]["mimeType"], "image/png");
    let view = &doc["bufferViews"][doc["images"][0]["bufferView"].as_u64().unwrap() as usize];
    assert_eq!(view["byteLength"], PNG.len());
    assert_eq!(doc["buffers"].as_array().unwrap().len(), 1);
    let mesh = Mesh::parse(&packed).unwrap();
    assert!(mesh.surfaces[0].maps.base.is_some());
    for refused in [
        "../paint.png",
        "https://example.com/paint.png",
        "textures/missing.png",
    ] {
        let bad = text.replace("textures/paint%20diff.png", refused);
        let error = gltf(bad.as_bytes(), read).unwrap_err();
        assert!(error.starts_with("images[0]: "), "{refused}: {error}");
    }
    let escape = String::from_utf8(data().read("triangle.gltf").unwrap())
        .unwrap()
        .replace("\"uri\":\"triangle.bin\"", "\"uri\":\"../triangle.bin\"");
    assert!(
        gltf(escape.as_bytes(), |path| data().read(path))
            .unwrap_err()
            .contains("outside the asset folder")
    );
}

#[test]
fn an_obj_becomes_one_node_per_object_with_flipped_uvs() {
    let folder = Source::folder(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/obj"));
    let quad = folder.read("quad.obj").unwrap();
    let packed = from_obj(&quad, "quad.obj", |path| folder.read(path)).unwrap();
    let mesh = Mesh::parse(&packed).unwrap();
    let names: Vec<&str> = mesh.nodes.iter().map(|n| n.name.as_str()).collect();
    assert_eq!(names, ["Quad", "Quad_face", "Tri_v_vn"]);
    let first = &mesh.groups[0].primitives[0];
    assert_eq!(
        first.positions,
        [
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0]
        ]
    );
    assert_eq!(first.uvs, [[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]]);
    assert_eq!(first.indices, [0, 1, 2, 0, 2, 3]);
    assert!(first.normals.is_empty());
    assert_eq!(mesh.groups[1].primitives[0].normals, [[0.0, 0.0, 1.0]; 4]);
    let cornell = folder.read("cornell_box.obj").unwrap();
    let error = from_obj(&cornell, "cornell_box.obj", |path| folder.read(path)).unwrap_err();
    assert!(error.starts_with("material red: "), "{error}");
    assert!(
        error.contains("this diffuse texture has spaces.jpg"),
        "{error}"
    );
    let missing = from_obj(
        b"mtllib gone.mtl\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n",
        "a.obj",
        |path| folder.read(path),
    )
    .unwrap_err();
    assert!(missing.starts_with("mtllib gone.mtl: "), "{missing}");
    assert_eq!(
        from_obj(b"v 0 0 0\nl 1 1\n", "a.obj", |path| folder.read(path)).unwrap_err(),
        "the OBJ has no faces"
    );
}

const PANEL_OBJ: &str = "mtllib looks/panel.mtl\n\
v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt 1 1\nvt 0 1\nvn 0 0 1\n\
o panel\nusemtl painted\nf 1/1/1 2/2/1 3/3/1 4/4/1\nusemtl glass\nf 1/1/1 3/3/1 4/4/1\n\
o lamp\nusemtl glow\nf 1//1 2//1 3//1\n";

const PANEL_MTL: &str = "newmtl painted\nKd 0.5 0.25 0.125\nPr 0.3\nPm 1\nPc 0.5\nPcr 0.2\n\
map_Kd -s 2 2 1 swatch8.png\nnorm -bm 0.5 swatch8.png\n\
newmtl glass\nKd 1 1 1\nKs 0.5 0.5 0.5\nNs 1000\nTr 0.75\n\
newmtl glow\nKd 0 0 0\nKs 0 0 0\nKe 4 2 0\n";

#[test]
fn mtl_materials_map_onto_pfx_pbr_materials() {
    let read = |path: &str| match path {
        "looks/panel.mtl" => Ok(PANEL_MTL.as_bytes().to_vec()),
        "looks/swatch8.png" => Ok(PNG.to_vec()),
        other => Err(format!("{other}: not here")),
    };
    let packed = from_obj(PANEL_OBJ.as_bytes(), "panel.obj", read).unwrap();
    let doc = json(&packed);
    assert_eq!(doc["images"].as_array().unwrap().len(), 1);
    let painted = &doc["materials"][0];
    assert_eq!(
        painted["pbrMetallicRoughness"]["baseColorFactor"],
        serde_json::json!([0.5, 0.25, 0.125, 1.0])
    );
    assert_eq!(
        painted["pbrMetallicRoughness"]["baseColorTexture"]["extensions"]["KHR_texture_transform"],
        serde_json::json!({ "offset": [0.0, -1.0], "scale": [2.0, 2.0] })
    );
    assert_eq!(painted["normalTexture"]["scale"], 0.5);
    assert!(
        painted["extensions"]
            .get("KHR_materials_specular")
            .is_none()
    );
    let glass = &doc["materials"][1];
    assert_eq!(glass["alphaMode"], "BLEND");
    assert_eq!(glass["pbrMetallicRoughness"]["baseColorFactor"][3], 0.25);
    let glow = &doc["materials"][2];
    assert_eq!(glow["emissiveFactor"], serde_json::json!([1.0, 0.5, 0.0]));
    assert_eq!(
        glow["extensions"]["KHR_materials_emissive_strength"]["emissiveStrength"],
        4.0
    );
    assert_eq!(
        glow["extensions"]["KHR_materials_specular"]["specularColorFactor"],
        serde_json::json!([0.0, 0.0, 0.0])
    );

    let mesh = Mesh::parse(&packed).unwrap();
    let names: Vec<&str> = mesh.nodes.iter().map(|n| n.name.as_str()).collect();
    assert_eq!(names, ["panel", "lamp"]);
    assert_eq!(mesh.groups[0].primitives.len(), 2);
    assert_eq!(mesh.groups[0].primitives[1].material, Some(1));
    let painted = &mesh.surfaces[0];
    assert_eq!(painted.material.base, [0.5, 0.25, 0.125]);
    assert!((painted.material.roughness - 0.3).abs() < 1e-6);
    assert_eq!(painted.material.metalness, 1.0);
    assert_eq!(painted.material.clearcoat, 0.5);
    assert!(painted.maps.base.is_some() && painted.maps.normal.is_some());
    assert!((mesh.surfaces[1].material.roughness - 0.211_43).abs() < 1e-4);
    assert_eq!(mesh.surfaces[2].material.roughness, 1.0);
    assert_eq!(mesh.surfaces[2].material.emission, [4.0, 2.0, 0.0]);
}

#[test]
fn shininess_turns_into_roughness_through_the_beckmann_width() {
    let material = |ks: Option<[f32; 3]>, ns: Option<f32>, pr: Option<f32>| crate::obj::Material {
        specular: ks,
        shininess: ns,
        roughness: pr,
        ..Default::default()
    };
    assert_eq!(roughness(&material(None, None, None)), 1.0);
    assert_eq!(roughness(&material(Some([0.0; 3]), Some(500.0), None)), 1.0);
    assert_eq!(
        roughness(&material(Some([1.0; 3]), Some(1000.0), Some(0.25))),
        0.25
    );
    for (ns, expected) in [
        (0.0, 1.0),
        (2.0, 0.840_896),
        (98.0, 0.376_06),
        (1000.0, 0.211_43),
    ] {
        let found = roughness(&material(Some([0.5; 3]), Some(ns), None));
        assert!((found - expected).abs() < 1e-4, "Ns {ns}: {found}");
    }
}
