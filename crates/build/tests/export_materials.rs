use pfx_build::Registry;
use pfx_build::gltf_io;
use pfx_materials::{AlphaMode, Family, Material};
use serde_json::Value;
use std::path::{Path, PathBuf};

const MATERIALS: &str = r#"
[materials.lamp]
family = "emissive"
base = [0.3, 0.02, 0.02]
roughness = 0.4
emission = [6.0, 0.2, 0.1]

[materials.lens]
family = "glass"
base = [0.9, 0.3, 0.3]
roughness = 0.03
transmission = 0.92
ior = 1.49
thickness = 0.006
absorption = 3.5

[materials.pane]
family = "glass"
base = [0.95, 0.97, 1.0]
roughness = 0.0
transmission = 1.0
ior = 1.52
thin_walled = true

[materials.body]
family = "lacquer"
base = [0.05, 0.2, 0.6]
roughness = 0.35
clearcoat = 1.0
clearcoat_roughness = 0.04

[materials.panel]
base = [0.8, 0.7, 0.6]
roughness = 0.6
base_map = "tex/panel.png"
normal_map = "tex/bumps.png"
normal_scale = 0.5
uv_scale = [2.0, 3.0]
uv_offset = [0.25, 0.5]
uv_rotation = 0.3

[materials.grille]
family = "metal"
base = [0.2, 0.2, 0.2]
roughness = 0.7
metalness = 0.8
alpha = 0.9
alpha_mode = "mask"
alpha_cutoff = 0.3
"#;

const OBJECTS: &str = r#"
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
name = "pane"
kind = "box"
size = [0.8, 0.3, 0.01]
at = [-0.4, 0.55, 0.0]
material = "pane"

[[object]]
name = "panel"
kind = "box"
size = [0.6, 0.4, 0.02]
at = [-0.6, 0.0, 0.52]
material = "panel"

[[object]]
name = "grille"
kind = "box"
size = [0.5, 0.2, 0.02]
at = [0.0, -0.2, 0.52]
material = "grille"
"#;

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tmp/test-export-materials")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("tex")).unwrap();
    dir
}

fn png(texel: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, 4, 4);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    let data: Vec<u8> = (0..16).flat_map(|i| texel(i % 4, i / 4)).collect();
    writer.write_image_data(&data).unwrap();
    drop(writer);
    out
}

fn recipe(dir: &Path) -> PathBuf {
    std::fs::write(
        dir.join("tex/panel.png"),
        png(|x, y| [x as u8 * 60, y as u8 * 60, 90, 255]),
    )
    .unwrap();
    std::fs::write(
        dir.join("tex/bumps.png"),
        png(|x, _| [128 + x as u8 * 10, 128, 250, 255]),
    )
    .unwrap();
    let path = dir.join("parts.toml");
    std::fs::write(
        &path,
        format!("recipe = 1\nname = \"parts\"\n{OBJECTS}{MATERIALS}"),
    )
    .unwrap();
    path
}

fn materials(path: &Path) -> std::collections::BTreeMap<String, Material> {
    let recipe = pfx_recipe::load(path).unwrap().recipe;
    let built = pfx_build::build_with(&recipe, path.parent().unwrap(), &Registry::new()).unwrap();
    built
        .recipe(&recipe)
        .materials
        .into_iter()
        .map(|(name, value)| (name, serde_json::from_value(value).unwrap()))
        .collect()
}

fn reimported(dir: &Path, glb: &[u8]) -> PathBuf {
    std::fs::write(dir.join("parts.glb"), glb).unwrap();
    let path = dir.join("again.toml");
    std::fs::write(
        &path,
        "recipe = 1\nname = \"again\"\n\n[[object]]\nname = \"parts\"\nkind = \"mesh\"\npath = \"parts.glb\"\n",
    )
    .unwrap();
    path
}

fn same(name: &str, field: &str, got: f32, want: f32) {
    assert!(
        (got - want).abs() <= 1e-4 * want.abs().max(1.0),
        "{name}.{field}: {got} after the round trip, {want} before"
    );
}

fn same3(name: &str, field: &str, got: [f32; 3], want: [f32; 3]) {
    for k in 0..3 {
        same(name, field, got[k], want[k]);
    }
}

fn agree(name: &str, got: &Material, want: &Material) {
    assert_eq!(got.family, want.family, "{name}.family");
    same3(name, "base", got.base, want.base);
    same(name, "alpha", got.alpha, want.alpha);
    same(name, "roughness", got.roughness, want.roughness);
    same(name, "metalness", got.metalness, want.metalness);
    same3(name, "emission", got.emission, want.emission);
    same(name, "transmission", got.transmission, want.transmission);
    same(name, "ior", got.ior, want.ior);
    same(name, "thickness", got.thickness, want.thickness);
    same(name, "absorption", got.absorption, want.absorption);
    assert_eq!(got.thin_walled, want.thin_walled, "{name}.thin_walled");
    same(name, "clearcoat", got.clearcoat, want.clearcoat);
    same(
        name,
        "clearcoat_roughness",
        got.clearcoat_roughness,
        want.clearcoat_roughness,
    );
    assert_eq!(got.alpha_mode, want.alpha_mode, "{name}.alpha_mode");
    if got.alpha_mode == AlphaMode::Mask {
        same(name, "alpha_cutoff", got.alpha_cutoff, want.alpha_cutoff);
    }
}

fn exported_material<'a>(json: &'a Value, name: &str) -> &'a Value {
    json["materials"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["name"] == name)
        .unwrap_or_else(|| panic!("the export has no material `{name}`"))
}

#[test]
fn an_exported_recipe_carries_its_materials_and_reads_back_field_by_field() {
    let dir = scratch("round-trip");
    let source = recipe(&dir);
    let before = materials(&source);
    let glb = gltf_io::export_glb(&source, &Registry::new(), None).unwrap();
    let text = gltf_io::split(&glb).unwrap().json;
    assert!(
        !String::from_utf8_lossy(text).contains("null"),
        "the export writes a null the glTF schema refuses"
    );
    let json: Value = serde_json::from_slice(text).unwrap();

    let lamp = exported_material(&json, "lamp");
    assert_eq!(lamp["emissiveFactor"][0], 1.0);
    assert_eq!(
        lamp["extensions"]["KHR_materials_emissive_strength"]["emissiveStrength"],
        6.0
    );
    let lens = exported_material(&json, "lens");
    assert_eq!(
        lens["extensions"]["KHR_materials_volume"]["thicknessFactor"],
        0.006
    );
    assert!(lens["extensions"]["KHR_materials_ior"]["ior"].is_number());
    let pane = exported_material(&json, "pane");
    assert!(pane["extensions"].get("KHR_materials_volume").is_none());
    let body = exported_material(&json, "body");
    assert_eq!(
        body["extensions"]["KHR_materials_clearcoat"]["clearcoatFactor"],
        1.0
    );
    let panel = exported_material(&json, "panel");
    let texture = &panel["pbrMetallicRoughness"]["baseColorTexture"];
    assert_eq!(
        texture["extensions"]["KHR_texture_transform"]["scale"],
        serde_json::json!([2.0, 3.0])
    );
    assert!(panel["normalTexture"]["extensions"]["KHR_texture_transform"].is_object());
    assert_eq!(panel["normalTexture"]["scale"], 0.5);
    let grille = exported_material(&json, "grille");
    assert_eq!(grille["alphaMode"], "MASK");
    assert_eq!(grille["alphaCutoff"], 0.3);
    let used = json["extensionsUsed"].as_array().unwrap();
    for name in [
        "KHR_materials_emissive_strength",
        "KHR_materials_transmission",
        "KHR_materials_volume",
        "KHR_materials_ior",
        "KHR_materials_clearcoat",
        "KHR_texture_transform",
    ] {
        assert!(
            used.iter().any(|u| u == name),
            "extensionsUsed lacks {name}"
        );
    }

    let after = materials(&reimported(&dir, &glb));
    for (name, want) in &before {
        let got = after
            .get(name)
            .unwrap_or_else(|| panic!("`{name}` is lost: {:?}", after.keys()));
        agree(name, got, want);
    }
    if let Some(Err(text)) = gltf_io::validator(&dir.join("parts.glb")) {
        panic!("the Khronos validator refuses the export: {text}");
    }
}

#[test]
fn one_exported_object_carries_the_same_materials_as_the_whole_recipe() {
    let dir = scratch("one-object");
    let source = recipe(&dir);
    let whole = gltf_io::export_glb(&source, &Registry::new(), None).unwrap();
    let one = gltf_io::export_glb(&source, &Registry::new(), Some("lens")).unwrap();
    let json = |glb: &[u8]| -> Value {
        serde_json::from_slice(gltf_io::split(glb).unwrap().json).unwrap()
    };
    let (whole, one) = (json(&whole), json(&one));
    assert_eq!(
        exported_material(&one, "lens"),
        exported_material(&whole, "lens")
    );
}

#[test]
fn a_recipe_document_exports_the_materials_of_its_recipe_not_its_cached_build() {
    let dir = scratch("document");
    let source = recipe(&dir);
    let doc = pfx_build::document::Document::pack(&source, &Registry::new()).unwrap();
    let packed = dir.join("parts.pitofx");
    doc.write(&packed).unwrap();
    let json = |glb: &[u8]| -> Value {
        serde_json::from_slice(gltf_io::split(glb).unwrap().json).unwrap()
    };
    let from_recipe = json(&gltf_io::export_glb(&source, &Registry::new(), None).unwrap());
    let from_document = json(&gltf_io::export_glb(&packed, &Registry::new(), None).unwrap());
    assert_eq!(from_document["materials"], from_recipe["materials"]);
    let lens = json(&gltf_io::export_glb(&packed, &Registry::new(), Some("lens")).unwrap());
    assert_eq!(
        exported_material(&lens, "lens"),
        exported_material(&from_recipe, "lens")
    );
}

#[test]
fn a_gltf_lens_without_a_volume_reads_as_thin_and_with_one_keeps_its_depth() {
    let glb = |extensions: Value| {
        let json = serde_json::json!({
            "asset": {"version": "2.0"},
            "materials": [{
                "name": "lens",
                "pbrMetallicRoughness": {"baseColorFactor": [1.0, 0.5, 0.5, 1.0], "metallicFactor": 0.0},
                "extensions": extensions,
            }],
        });
        gltf_io::join(&serde_json::to_vec(&json).unwrap(), &[])
    };
    let thin = glb(serde_json::json!({"KHR_materials_transmission": {"transmissionFactor": 1.0}}));
    let solid = glb(serde_json::json!({
        "KHR_materials_transmission": {"transmissionFactor": 1.0},
        "KHR_materials_volume": {"thicknessFactor": 0.01, "attenuationDistance": 0.5, "attenuationColor": [1.0, 0.5, 0.5]},
    }));
    let read = |glb: &[u8]| -> Material {
        let mesh = pfx_load::Mesh::parse(glb).unwrap();
        mesh.surfaces[0].material
    };
    let thin = read(&thin);
    assert!(thin.thin_walled);
    let solid = read(&solid);
    assert!(!solid.thin_walled);
    same("lens", "thickness", solid.thickness, 0.01);
    let want = -(0.5f32.ln()) * 2.0 * 2.0 / (0.5 + 0.5);
    same("lens", "absorption", solid.absorption, want);
    assert_eq!(solid.family, Family::Plain);
}

#[test]
fn a_metallic_transmissive_material_earns_an_import_note_naming_it() {
    let json = serde_json::json!({
        "asset": {"version": "2.0"},
        "materials": [
            {"name": "Chromed", "pbrMetallicRoughness": {"metallicFactor": 1.0},
             "extensions": {"KHR_materials_transmission": {"transmissionFactor": 0.96}}},
            {"name": "Clear", "pbrMetallicRoughness": {"metallicFactor": 0.0},
             "extensions": {"KHR_materials_transmission": {"transmissionFactor": 0.96}}},
        ],
    });
    let notes = gltf_io::notes(&gltf_io::join(&serde_json::to_vec(&json).unwrap(), &[]));
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(notes[0].contains("`Chromed`"), "{}", notes[0]);
}
