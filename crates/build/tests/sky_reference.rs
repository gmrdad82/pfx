use pfx_build::Registry;
use pfx_build::document::Document;
use std::path::{Path, PathBuf};

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("sky_reference")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn hdr() -> Vec<u8> {
    let mut bytes = b"#?RADIANCE\nFORMAT=32-bit_rle_rgbe\n\n-Y 1 +X 2\n".to_vec();
    bytes.extend([128, 128, 128, 129, 128, 128, 128, 129]);
    bytes
}

fn plaque(dir: &Path) -> PathBuf {
    std::fs::create_dir_all(dir.join("assets/sky")).unwrap();
    std::fs::write(dir.join("assets/sky/softbox.hdr"), hdr()).unwrap();
    let recipe = dir.join("plaque.toml");
    std::fs::write(
        &recipe,
        "recipe = 1\nname = \"plaque\"\n\n[sky]\nkind = \"hdr\"\npath = \"assets/sky/softbox.hdr\"\nsource = \"generated\"\n\n[[object]]\nname = \"b\"\nkind = \"box\"\nsize = [1.0, 1.0, 1.0]\n",
    )
    .unwrap();
    recipe
}

fn packed_hdr(doc: &Document) -> bool {
    doc.files
        .keys()
        .any(|p| p.to_string_lossy().ends_with("softbox.hdr"))
}

fn lit(pfx: &Path) -> Result<(), String> {
    let doc = Document::open(pfx)?;
    let (recipe, _mount) = doc.mount(pfx)?;
    let loaded = pfx_recipe::load(&recipe).map_err(|e| e.to_string())?;
    pfx_recipe_scene::Lighting::of(&loaded.recipe)
        .map(|_| ())
        .map_err(|e| e.0)
}

#[test]
fn an_hdri_inside_the_recipes_folder_stays_a_file_beside_the_pack() {
    let dir = scratch("inside");
    let recipe = plaque(&dir);
    let out = dir.join("plaque.pitofx");
    let doc = Document::pack_for(&recipe, &Registry::new(), &out).unwrap();
    assert!(!packed_hdr(&doc), "{:?}", doc.files.keys());
    assert_eq!(doc.referenced, ["assets/sky/softbox.hdr"]);
    assert!(doc.outside.is_empty(), "{:?}", doc.outside);
    doc.write(&out).unwrap();
    let opened = Document::open(&out).unwrap();
    assert!(!packed_hdr(&opened));
    let beside: Vec<String> = opened
        .beside(&out)
        .unwrap()
        .into_iter()
        .map(|(name, file)| {
            assert_eq!(std::fs::read(file).unwrap(), hdr());
            name
        })
        .collect();
    assert_eq!(beside, ["assets/sky/softbox.hdr"]);
    lit(&out).unwrap();

    let away = scratch("away");
    let copied = away.join("plaque.pitofx");
    std::fs::copy(&out, &copied).unwrap();
    assert!(
        Document::open(&copied)
            .unwrap()
            .beside(&copied)
            .unwrap()
            .is_empty()
    );
    let said = lit(&copied).unwrap_err();
    assert!(said.starts_with("sky.path:"), "{said}");
}

#[test]
fn an_hdri_is_packed_when_the_pack_cannot_read_it_beside_itself() {
    let dir = scratch("elsewhere");
    let recipe = plaque(&dir);
    let packed = Document::pack(&recipe, &Registry::new()).unwrap();
    assert!(packed_hdr(&packed));
    assert!(packed.referenced.is_empty());
    let other = scratch("elsewhere-out");
    let out = other.join("plaque.pitofx");
    let doc = Document::pack_for(&recipe, &Registry::new(), &out).unwrap();
    assert!(packed_hdr(&doc), "{:?}", doc.files.keys());
    assert!(doc.referenced.is_empty());
    doc.write(&out).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    lit(&out).unwrap();
}

#[test]
fn a_copy_of_the_folder_keeps_the_reference_when_it_saves_back_beside_the_same_hdri() {
    let dir = scratch("source");
    plaque(&dir);
    let out = dir.join("plaque.pitofx");
    let work = scratch("work");
    let recipe = plaque(&work);
    let doc = Document::pack_for(&recipe, &Registry::new(), &out).unwrap();
    assert_eq!(doc.referenced, ["assets/sky/softbox.hdr"]);
    std::fs::write(dir.join("assets/sky/softbox.hdr"), b"changed").unwrap();
    let doc = Document::pack_for(&recipe, &Registry::new(), &out).unwrap();
    assert!(packed_hdr(&doc));
    assert!(doc.referenced.is_empty());
}

#[test]
fn a_sky_path_naming_a_file_that_is_not_an_hdr_is_never_kept_beside() {
    let dir = scratch("not-a-sky");
    let recipe = plaque(&dir);
    let out = dir.join("plaque.pitofx");
    let doc = Document::pack_for(&recipe, &Registry::new(), &out).unwrap();
    assert_eq!(doc.referenced, ["assets/sky/softbox.hdr"]);
    doc.write(&out).unwrap();
    std::fs::write(dir.join("assets/sky/softbox.hdr"), b"not a sky").unwrap();
    let opened = Document::open(&out).unwrap();
    assert!(opened.beside(&out).unwrap().is_empty());
    let doc = Document::pack_for(&recipe, &Registry::new(), &out).unwrap();
    assert!(doc.referenced.is_empty());
}
