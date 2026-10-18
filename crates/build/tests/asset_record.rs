use pfx_build::Registry;
use pfx_build::document::{ASSET_RECORD, Document};
use pfx_build::gltf_io;
use pfx_file::{Codec, Writer};
use std::path::{Path, PathBuf};

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("asset_record")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn record(dir: &Path, name: &str) {
    std::fs::create_dir_all(dir).unwrap();
    Writer::new()
        .chunk(
            pfx_file::FACT,
            format!("licence = \"CC0-1.0\"\ncredit = \"{name}\"\n[asset]\nname = \"{name}\"\n")
                .into_bytes(),
            Codec::Auto,
        )
        .write(dir.join(ASSET_RECORD))
        .unwrap();
}

#[test]
fn a_packed_scene_carries_the_record_of_each_fetched_asset_it_reads() {
    let dir = scratch("pack");
    let source = dir.join("box.toml");
    std::fs::write(
        &source,
        "recipe = 1\nname = \"box\"\n\n[[object]]\nname = \"b\"\nkind = \"box\"\nsize = [1.0, 1.0, 1.0]\n",
    )
    .unwrap();
    let glb = gltf_io::export_glb(&source, &Registry::new(), None).unwrap();
    let used = dir.join("assets/crate");
    record(&used, "Crate");
    std::fs::create_dir_all(used.join("Models")).unwrap();
    std::fs::write(used.join("Models/crate.glb"), glb).unwrap();
    record(&dir.join("assets/unused"), "Unused");
    let scene = dir.join("scene.toml");
    std::fs::write(
        &scene,
        "recipe = 1\nname = \"scene\"\n\n[[object]]\nname = \"crate\"\nkind = \"mesh\"\npath = \"assets/crate/Models/crate.glb\"\n",
    )
    .unwrap();
    let doc = Document::pack(&scene, &Registry::new()).unwrap();
    let names: Vec<String> = doc
        .files
        .keys()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .collect();
    assert!(
        names.contains(&"assets/crate/asset.pitofx".to_string()),
        "{names:?}"
    );
    assert!(
        names.contains(&"assets/crate/Models/crate.glb".to_string()),
        "{names:?}"
    );
    assert!(!names.iter().any(|n| n.contains("unused")), "{names:?}");
}
