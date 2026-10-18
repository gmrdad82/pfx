use std::path::Path;

const PORTS: [(&str, &str); 20] = [
    ("crates/load/src/uri.rs", "cgltf"),
    ("crates/load/src/obj.rs", "tobj"),
    ("crates/load/src/obj.rs", "tinyobjloader"),
    ("crates/motion/src/route.rs", "Godot Engine"),
    ("crates/mesh/src/simplify.rs", "meshoptimizer"),
    ("crates/mesh/src/tangents.rs", "MikkTSpace"),
    ("crates/build/src/builtin/loft.rs", "curvo"),
    ("crates/build/src/builtin/loft.rs", "OpenSubdiv"),
    ("crates/generators/src/pyrandom.rs", "Matsumoto"),
    ("crates/generators/src/numpy_random.rs", "NumPy"),
    ("crates/load/src/jpeg/mod.rs", "zune-jpeg"),
    ("crates/load/src/jpeg/headers.rs", "zune-jpeg"),
    ("crates/load/src/jpeg/components.rs", "zune-jpeg"),
    ("crates/load/src/jpeg/huffman.rs", "zune-jpeg"),
    ("crates/load/src/jpeg/bitstream.rs", "zune-jpeg"),
    ("crates/load/src/jpeg/mcu.rs", "zune-jpeg"),
    ("crates/load/src/jpeg/progressive.rs", "zune-jpeg"),
    ("crates/load/src/jpeg/idct.rs", "zune-jpeg"),
    ("crates/load/src/jpeg/upsample.rs", "zune-jpeg"),
    ("crates/load/src/jpeg/color.rs", "zune-jpeg"),
];

fn read(name: &str) -> String {
    std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(name)).unwrap()
}

#[test]
fn every_ported_file_is_credited_in_notice_and_inspiration() {
    let notice = read("NOTICE.md");
    let inspiration = read("INSPIRATION.md");
    for (file, origin) in PORTS {
        assert!(
            Path::new(env!("CARGO_MANIFEST_DIR")).join(file).is_file(),
            "{file} is gone; drop its credit"
        );
        let short = file.rsplit('/').next().unwrap_or(file);
        assert!(
            notice.contains(file) || notice.contains(short),
            "NOTICE.md omits {file}"
        );
        assert!(notice.contains(origin), "NOTICE.md omits {origin}");
        assert!(inspiration.contains(short), "INSPIRATION.md omits {short}");
    }
}
