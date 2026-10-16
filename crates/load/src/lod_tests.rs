use std::path::{Path, PathBuf};

use crate::fixture::{LOD_RINGS, lod_error, lod_glb, lod_sphere};
use crate::scene::{COVERAGE_HEIGHT, Scene};

struct Folder {
    root: PathBuf,
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn open(name: &str, glb: &[u8]) -> Result<Scene, String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tmp/lod-tests")
        .join(name);
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let folder = Folder { root };
    std::fs::write(folder.root.join("ball.glb"), glb).unwrap();
    let path = folder.root.join("ball.scene.toml");
    std::fs::write(
        &path,
        "format = 1\n\n[mesh.ball]\nfile = \"ball.glb\"\n\n[[object]]\nname = \"ball\"\nmesh = \"ball\"\n",
    )
    .unwrap();
    Scene::open(&path).map_err(String::from)
}

fn triangles(rings: u32) -> usize {
    (rings as usize * 2 - 2) * rings as usize * 2
}

#[test]
fn a_node_with_two_lod_nodes_loads_as_one_part_with_three_levels() {
    let scene = open("errors", &lod_glb("ball", None, true)).unwrap();
    let mesh = &scene.meshes["ball"];
    assert_eq!(mesh.parts.len(), 1);
    assert_eq!(mesh.parts[0].node, "ball");
    let geometry = &mesh.parts[0].geometry;
    assert_eq!(geometry.indices.len() / 3, triangles(LOD_RINGS[0]));
    let levels: Vec<(usize, f32)> = geometry
        .levels
        .iter()
        .map(|level| (level.geometry.indices.len() / 3, level.error))
        .collect();
    assert_eq!(
        levels,
        [
            (triangles(LOD_RINGS[1]), lod_error(LOD_RINGS[1])),
            (triangles(LOD_RINGS[2]), lod_error(LOD_RINGS[2])),
        ]
    );
    let draws = scene.draws();
    assert_eq!(draws.items.len(), 1);
    assert_eq!(draws.items[0].geometry.levels.len(), 2);
}

#[test]
fn screen_coverage_stands_in_for_a_missing_error() {
    let scene = open("coverage", &lod_glb("ball", Some(&[0.5, 0.2, 0.01]), false)).unwrap();
    let levels = &scene.meshes["ball"].parts[0].geometry.levels;
    let (positions, ..) = lod_sphere(LOD_RINGS[0]);
    let extent = |axis: usize| {
        let values = positions.iter().map(|p| p[axis]);
        values.clone().fold(f32::NEG_INFINITY, f32::max) - values.fold(f32::INFINITY, f32::min)
    };
    let radius = (0..3).map(|axis| extent(axis).powi(2)).sum::<f32>().sqrt() * 0.5;
    assert_eq!(levels.len(), 2);
    for (level, coverage) in levels.iter().zip([0.5, 0.2]) {
        let expected = radius / (coverage * COVERAGE_HEIGHT);
        assert!(
            (level.error - expected).abs() < 1e-7,
            "{} {expected}",
            level.error
        );
    }
}

#[test]
fn a_lod_node_without_an_error_or_a_coverage_is_refused() {
    let error = open("bare", &lod_glb("ball", None, false)).unwrap_err();
    assert!(error.contains("no extras.lod.error"), "{error}");
}
