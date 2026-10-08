use std::path::{Path, PathBuf};

use pfx_core::anim::fixture;

use crate::Mesh;
use crate::scene::Scene;

fn column_mesh(bones: usize) -> Mesh {
    Mesh::parse(&crate::fixture::skinned_column(bones)).unwrap()
}

#[test]
fn a_skinned_glb_keeps_its_skin_joints_weights_and_animations() {
    let mesh = column_mesh(3);
    assert_eq!(mesh.skins.len(), 1);
    assert_eq!(mesh.skins[0].joints, [0, 1, 2]);
    assert_eq!(mesh.nodes[3].skin, Some(0));
    assert_eq!(mesh.nodes[1].trs.translation, [0.0, 1.0, 0.0]);
    let primitive = &mesh.groups[0].primitives[0];
    let column = fixture::column(3);
    assert_eq!(primitive.joints.as_deref(), Some(column.joints.as_slice()));
    assert_eq!(
        primitive.weights.as_deref(),
        Some(column.weights.as_slice())
    );
    let names: Vec<&str> = mesh.animations.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(names, ["bend", "wave", "lift", "twist"]);
}

#[test]
fn the_rig_of_a_skin_matches_the_engine_fixture() {
    let mesh = column_mesh(3);
    let found = mesh.rig(0).unwrap();
    assert_eq!(found.remap, [0, 1, 2]);
    let expected = fixture::rig(3);
    assert_eq!(found.rig.skeleton(), expected.skeleton());
    assert_eq!(
        found.rig.clips(),
        expected
            .clips()
            .iter()
            .map(|clip| {
                pfx_core::anim::Clip::new(clip.name(), clip.channels().to_vec()).unwrap()
            })
            .collect::<Vec<_>>()
    );
    assert!(mesh.rig(1).is_err());
}

#[test]
fn joints_listed_children_first_are_reordered_and_vertices_remapped() {
    let mut mesh = column_mesh(3);
    mesh.skins[0].joints = vec![2, 0, 1];
    mesh.skins[0].inverse_binds.rotate_right(1);
    let found = mesh.rig(0).unwrap();
    assert_eq!(found.remap, [2, 0, 1]);
    let names: Vec<&str> = found
        .rig
        .skeleton()
        .joints()
        .iter()
        .map(|joint| joint.name.as_str())
        .collect();
    assert_eq!(names, ["bone0", "bone1", "bone2"]);
    let joints = found
        .joints(&[[0, 1, 2, 0]], &[[0.5, 0.25, 0.25, 0.0]])
        .unwrap();
    assert_eq!(joints, [[2, 0, 1, 0]]);
    assert!(
        found
            .joints(&[[7, 0, 0, 0]], &[[1.0, 0.0, 0.0, 0.0]])
            .is_err()
    );
}

struct Folder {
    root: PathBuf,
}

impl Folder {
    fn new(name: &str) -> Self {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tmp/rig-tests")
            .join(name);
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        Self { root }
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

const SCENE: &str = r#"format = 1

[mesh.column]
file = "column.glb"
id = "skncm00001"

[[object]]
name = "post"
mesh = "column"
at = [1.0, 0.0, 0.0]
id = "sknbj00001"
"#;

#[test]
fn a_scene_mesh_carries_its_rig_and_its_parts_carry_their_skin() {
    let folder = Folder::new("skinned scene");
    std::fs::write(
        folder.root.join("column.glb"),
        crate::fixture::skinned_column(2),
    )
    .unwrap();
    let path = folder.root.join("room.scene.toml");
    std::fs::write(&path, SCENE).unwrap();
    let scene = Scene::open(&path).unwrap();
    let mesh = &scene.meshes["column"];
    let rig = mesh.rig.as_ref().expect("the mesh has a rig");
    assert_eq!(rig.skeleton().len(), 2);
    assert_eq!(rig.clip("bend"), Some(0));
    assert_eq!(mesh.parts.len(), 1);
    let part = &mesh.parts[0];
    assert_eq!(part.transform, crate::scene::IDENTITY);
    let skin = part.geometry.skin.as_ref().expect("the part is skinned");
    assert_eq!(skin.joints, fixture::column(2).joints);
    assert_eq!(skin.palette(), 2);
    let draws = scene.draws();
    assert_eq!(draws.items[0].model[3], [1.0, 0.0, 0.0, 1.0]);
    let plain = crate::scene::Geometry::new(
        part.geometry.positions.clone(),
        part.geometry.normals.clone(),
        part.geometry.tangents.clone(),
        part.geometry.uvs.clone(),
        None,
        part.geometry.indices.clone(),
    );
    assert_ne!(plain.hash, part.geometry.hash);
}

#[test]
fn an_animated_object_is_dynamic_and_a_plain_one_is_not() {
    let folder = Folder::new("dynamic animation");
    std::fs::write(
        folder.root.join("column.glb"),
        crate::fixture::skinned_column(2),
    )
    .unwrap();
    let path = folder.root.join("room.scene.toml");
    std::fs::write(&path, SCENE).unwrap();
    let scene = Scene::open(&path).unwrap();
    assert!(!scene.dynamic(0));
    std::fs::write(
        &path,
        format!("{SCENE}\n[object.animation]\nclip = \"bend\"\n"),
    )
    .unwrap();
    let scene = Scene::open(&path).unwrap();
    assert!(scene.dynamic(0));
    assert_eq!(scene.animations["post"].clip.as_deref(), Some("bend"));
    assert!(scene.static_subset().objects[0].hidden);
}
