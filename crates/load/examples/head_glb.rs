use std::path::PathBuf;

fn main() {
    let path = std::env::args().nth(1).map_or_else(
        || PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/head.glb"),
        PathBuf::from,
    );
    let bytes = pfx_load::fixture::head();
    if let Err(error) = std::fs::write(&path, &bytes) {
        eprintln!("head_glb: {}: {error}", path.display());
        std::process::exit(1);
    }
    println!("{} ({} bytes)", path.display(), bytes.len());
}
