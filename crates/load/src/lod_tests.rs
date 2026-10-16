use std::path::{Path, PathBuf};

use crate::fixture::{
    LOD_RINGS, LOD_TARGETS, lod_error, lod_glb, lod_group_glb, lod_morph_glb, lod_sphere,
};
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

#[test]
fn a_morphed_node_takes_morphed_levels_with_its_own_targets_and_morph_slot() {
    let scene = open("morphed", &lod_morph_glb("ball", &LOD_TARGETS)).unwrap();
    let mesh = &scene.meshes["ball"];
    assert_eq!(mesh.parts.len(), 1);
    let geometry = &mesh.parts[0].geometry;
    let base = geometry.morph.as_ref().expect("the base is morphed");
    assert_eq!(base.targets.len(), 2);
    assert_eq!(geometry.levels.len(), 2);
    for level in &geometry.levels {
        let morph = level
            .geometry
            .morph
            .as_ref()
            .expect("a level of a morphed node is morphed");
        assert_eq!(morph.morph, base.morph);
        assert_eq!(morph.targets.len(), 2);
        assert!(
            morph
                .targets
                .iter()
                .all(|target| target.positions.len() == level.geometry.positions.len())
        );
    }
    let rig = mesh.rig.as_ref().expect("a morphed mesh has a rig");
    assert_eq!(rig.morphs().len(), 1);
}

#[test]
fn a_level_whose_targets_differ_from_its_node_is_refused() {
    for (name, targets) in [
        ("renamed", &["bulge", "swell"][..]),
        ("fewer", &["bulge"][..]),
    ] {
        let error = open(name, &lod_morph_glb("ball", targets)).unwrap_err();
        assert!(error.contains("morph targets"), "{error}");
    }
}

#[test]
fn a_beard_level_naming_its_head_carries_the_group_and_the_rest_carry_none() {
    let scene = open("grouped", &lod_group_glb(Some("head"), false)).unwrap();
    let groups: Vec<(&str, Option<&str>, usize)> = scene.meshes["ball"]
        .parts
        .iter()
        .map(|part| {
            (
                part.node.as_str(),
                part.geometry.group.as_deref(),
                part.geometry.levels.len(),
            )
        })
        .collect();
    assert_eq!(
        groups,
        [
            ("head", None, 2),
            ("beard", Some("head"), 1),
            ("brow", None, 0)
        ]
    );
    let plain = open("ungrouped", &lod_group_glb(None, false)).unwrap();
    assert!(
        plain.meshes["ball"]
            .parts
            .iter()
            .all(|part| part.geometry.group.is_none())
    );
}

#[test]
fn a_group_naming_a_missing_node_or_one_without_levels_is_refused_with_the_file() {
    for (group, wanted) in [
        ("nose", "which is not in the file"),
        ("brow", "which has no MSFT_lod levels"),
    ] {
        let error = open(group, &lod_group_glb(Some(group), false)).unwrap_err();
        assert!(error.contains(wanted), "{error}");
        assert!(error.contains("ball.glb"), "{error}");
        assert!(
            error.contains(&format!("node beard: extras.lod.group names node {group}")),
            "{error}"
        );
    }
}
