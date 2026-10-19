use std::path::{Path, PathBuf};

use super::*;

fn open(name: &str, particles: &str) -> Result<Scene, SceneError> {
    let root: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tmp/scene-tests")
        .join(name);
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/scenes");
    std::fs::copy(fixtures.join("block.gltf"), root.join("block.gltf")).unwrap();
    let path = root.join("spray.scene.toml");
    std::fs::write(
        &path,
        format!(
            "[mesh.block]\nfile = \"block.gltf\"\n\n[[object]]\nname = \"car\"\nmesh = \"block\"\nat = [1.0, 0.0, 0.0]\n\n{particles}"
        ),
    )
    .unwrap();
    let scene = Scene::open(&path);
    let _ = std::fs::remove_dir_all(&root);
    scene
}

#[test]
fn particles_attach_to_an_object_or_its_node_and_refuse_what_names_none() {
    let scene = open(
        "particles-node",
        "[[particles]]\nname = \"spray\"\nparent = \"car\"\nnode = \"Block\"\nat = [0.0, -0.3, -0.1]\nrate = 50.0\nrate_keys = [[0.0, 0.0], [2.0, 400.0]]\ncolours = [[0.0, \"#ffffff\"], [1.0, \"#000000\"]]\nblend = \"add\"\n",
    )
    .unwrap();
    let spray = &scene.particles["spray"];
    let attach = spray.attach.as_ref().unwrap();
    assert_eq!(attach.object, "car");
    assert_eq!(attach.node.as_ref().unwrap().name, "Block");
    assert_eq!(spray.at, [0.0, -0.3, -0.1]);
    let recipe = &spray.recipe;
    assert_eq!(recipe.rate_at(1.0), 200.0);
    assert_eq!(recipe.colour.sample(1.0), [0.0; 3]);
    assert_eq!(recipe.blend, pfx_core::sim::particles::Blend::Add);
    let world = attach.world(scene.objects[0].model, None);
    assert_eq!(world[3][0], 1.0);
    let unknown = open(
        "particles-ghost",
        "[[particles]]\nname = \"spray\"\nparent = \"ghost\"\n",
    )
    .unwrap_err();
    assert!(unknown.message.contains("ghost"), "{unknown}");
    let missing = open(
        "particles-wheel",
        "[[particles]]\nname = \"spray\"\nparent = \"car\"\nnode = \"wheel_rl\"\n",
    )
    .unwrap_err();
    assert!(missing.message.contains("wheel_rl"), "{missing}");
}

#[test]
fn a_prewarmed_spray_is_formed_at_time_zero_and_refuses_a_prewarm_past_its_cap() {
    let scene = open(
        "prewarm",
        "[[particles]]\nname = \"steam\"\nparent = \"car\"\nrate = 30.0\nlifetime = [2.0, 2.0]\nprewarm = 1.5\n",
    )
    .unwrap();
    assert_eq!(scene.particles["steam"].recipe.prewarm, 1.5);
    let mut sprays = super::sprays::Sprays::of(&scene);
    sprays.seek_posed(0.0, |_| None);
    let formed = sprays.live();
    assert!((40..=50).contains(&formed), "{formed}");
    sprays.seek_posed(1.0, |_| None);
    sprays.seek_posed(0.0, |_| None);
    assert_eq!(sprays.live(), formed);
    assert_eq!(
        pfx_core::sim::particles::PREWARM_MAX,
        pfx_scene::types::PREWARM_MAX
    );
    let refused = open(
        "prewarm-long",
        "[[particles]]\nname = \"steam\"\nprewarm = 31.0\n",
    );
    assert!(refused.is_err());
}
