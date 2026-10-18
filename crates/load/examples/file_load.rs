use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use pfx_load::Mesh;
use pfx_load::fixture::{pfx_from_glb, plain_glb};

const WARM_RUNS: usize = 7;

fn evict(path: &Path) -> bool {
    Command::new("dd")
        .arg(format!("if={}", path.display()))
        .args(["iflag=nocache", "count=0", "status=none"])
        .status()
        .is_ok_and(|status| status.success())
}

fn load(path: &Path) -> f64 {
    let start = Instant::now();
    let bytes = std::fs::read(path).expect("read");
    let mesh = Mesh::parse(&bytes).expect("parse");
    let took = start.elapsed().as_secs_f64() * 1000.0;
    assert!(!mesh.groups.is_empty());
    took
}

fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

fn measure(path: &Path) -> (f64, f64) {
    let mut cold = Vec::new();
    for _ in 0..3 {
        if !evict(path) {
            eprintln!(
                "file_load: cannot drop {} from the page cache",
                path.display()
            );
        }
        cold.push(load(path));
    }
    load(path);
    let warm = (0..WARM_RUNS).map(|_| load(path)).collect();
    (median(cold), median(warm))
}

fn main() {
    let out = std::env::args().nth(1).map_or_else(
        || PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tmp/pfx-bench"),
        PathBuf::from,
    );
    std::fs::create_dir_all(&out).expect("output folder");
    for (name, rings) in [("car", 200u32), ("scene", 1000)] {
        let glb = plain_glb(name, rings);
        let glb_path = out.join(format!("{name}.glb"));
        let lz4_path = out.join(format!("{name}.pitofx"));
        let store_path = out.join(format!("{name}.store.pitofx"));
        std::fs::write(&glb_path, &glb).expect("write glb");
        std::fs::write(&lz4_path, pfx_from_glb(&glb, &[], true)).expect("write pfx");
        std::fs::write(&store_path, pfx_from_glb(&glb, &[], false)).expect("write pfx");
        let triangles: usize = Mesh::parse(&glb)
            .expect("parse")
            .groups
            .iter()
            .flat_map(|group| &group.primitives)
            .map(|primitive| primitive.indices.len() / 3)
            .sum();
        for (kind, path) in [
            ("glb", &glb_path),
            ("pfx-lz4", &lz4_path),
            ("pfx-store", &store_path),
        ] {
            let size = std::fs::metadata(path).expect("size").len();
            let (cold, warm) = measure(path);
            println!(
                "{{\"model\":\"{name}\",\"triangles\":{triangles},\"file\":\"{kind}\",\"bytes\":{size},\"cold_ms\":{cold:.2},\"warm_ms\":{warm:.2}}}"
            );
        }
    }
}
