use pfx_build::{Document, Registry, build_with, gltf_io};
use pfx_recipe::{Pick, apply_variant, load};
use std::path::{Path, PathBuf};

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("variant_pack")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn glb(dir: &Path, name: &str, size: f64) -> Vec<u8> {
    let source = dir.join(format!("{name}.toml"));
    std::fs::write(
        &source,
        format!(
            "recipe = 1\nname = \"{name}\"\n\n[[object]]\nname = \"{name}\"\nkind = \"box\"\nsize = [{size}, {size}, {size}]\n"
        ),
    )
    .unwrap();
    gltf_io::export_glb(&source, &Registry::new(), None).unwrap()
}

const SCENE: &str = r#"recipe = 1
name = "scene"

[[object]]
name = "body"
kind = "mesh"
path = "mesh/body.glb"

[variant.tee]
set = { "object.body.path" = "mesh/body-tee.glb" }

[variant.coat]
set = { "object.body.path" = "mesh/body-coat.glb" }
"#;

#[test]
fn a_packed_scene_carries_every_mesh_its_variants_switch_to() {
    let dir = scratch("meshes");
    let parts = dir.join("parts");
    std::fs::create_dir_all(&parts).unwrap();
    let source = dir.join("source");
    let mesh = source.join("mesh");
    std::fs::create_dir_all(&mesh).unwrap();
    for (name, size) in [("body", 1.0), ("body-tee", 2.0), ("body-coat", 3.0)] {
        std::fs::write(mesh.join(format!("{name}.glb")), glb(&parts, name, size)).unwrap();
    }
    let scene = source.join("scene.toml");
    std::fs::write(&scene, SCENE).unwrap();
    let doc = Document::pack(&scene, &Registry::new()).unwrap();
    let names: Vec<String> = doc
        .files
        .keys()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .collect();
    for wanted in ["mesh/body.glb", "mesh/body-tee.glb", "mesh/body-coat.glb"] {
        assert!(names.contains(&wanted.to_string()), "{names:?}");
    }
    let folder = PathBuf::from("tmp/variant_pack");
    std::fs::create_dir_all(&folder).unwrap();
    let pfx = folder.join("scene.pitofx");
    doc.write(&pfx).unwrap();
    std::fs::remove_dir_all(&source).unwrap();
    let opened = Document::open(&pfx).unwrap();
    let (recipe, _mount) = opened.mount(&pfx).unwrap();
    let loaded = load(&recipe).unwrap();
    let base = recipe.parent().unwrap().to_path_buf();
    for name in ["tee", "coat"] {
        let variant = apply_variant(&loaded.recipe, name).unwrap();
        build_with(&variant, &base, &Registry::new())
            .unwrap_or_else(|e| panic!("variant {name}: {e}"));
        pfx_recipe_scene::load_at(
            &recipe,
            Some(name),
            &[],
            &Pick::default(),
            &Registry::new(),
            None,
        )
        .unwrap_or_else(|e| panic!("variant {name} plans a render: {e}"));
    }
}

#[test]
fn a_document_written_beside_a_recipe_that_includes_a_sibling_folder_renders_each_variant_alone() {
    let dir = scratch("beside");
    let parts = dir.join("parts");
    std::fs::create_dir_all(&parts).unwrap();
    let doc_dir = dir.join("street");
    let mesh = doc_dir.join("mesh");
    let shared = dir.join("shared");
    std::fs::create_dir_all(&mesh).unwrap();
    std::fs::create_dir_all(&shared).unwrap();
    for (name, size) in [("body", 1.0), ("body-tee", 2.0), ("body-coat", 3.0)] {
        std::fs::write(mesh.join(format!("{name}.glb")), glb(&parts, name, size)).unwrap();
    }
    std::fs::write(
        shared.join("rig.toml"),
        "[[camera]]\nname = \"main\"\nposition = [0.0, 2.0, 6.0]\ntarget = [0.0, 0.0, 0.0]\nfov_y_deg = 30.0\n",
    )
    .unwrap();
    let scene = doc_dir.join("street.toml");
    std::fs::write(
        &scene,
        SCENE.replace(
            "name = \"scene\"\n",
            "name = \"scene\"\ninclude = [\"../shared/rig.toml\"]\n",
        ),
    )
    .unwrap();
    let doc = Document::pack(&scene, &Registry::new()).unwrap();
    let pfx = doc_dir.join("street.pitofx");
    doc.write(&pfx).unwrap();
    std::fs::remove_dir_all(&mesh).unwrap();
    std::fs::remove_dir_all(&shared).unwrap();
    std::fs::remove_file(&scene).unwrap();
    let opened = Document::open(&pfx).unwrap();
    let (recipe, _mount) = opened.mount(&pfx).unwrap();
    for name in ["tee", "coat"] {
        pfx_recipe_scene::load_at(
            &recipe,
            Some(name),
            &[],
            &Pick::default(),
            &Registry::new(),
            None,
        )
        .unwrap_or_else(|e| panic!("variant {name} plans a render: {e}"));
    }
}
