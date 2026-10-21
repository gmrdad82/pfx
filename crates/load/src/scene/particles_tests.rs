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

const LEAVES: &str = "[object.wake]\nstrength = 0.7\nat = [0.0, 0.2, 0.0]\n\n[[object]]\nname = \"leaf\"\nmesh = \"block\"\nat = [5.0, 2.0, 0.0]\nscale = 0.1\n\n[[particles]]\nname = \"leaves\"\nmesh = \"leaf\"\nrate = 0.0\nlifetime = [60.0, 60.0]\nsize = [1.0, 1.0]\nalpha = [[0.0, 1.0], [1.0, 1.0]]\ngravity = [0.0, -9.81, 0.0]\nground = 0.0\nmass = 0.002\narea = 0.004\nplaced = [[-4.0, 0.0, 0.2, 0.0], [-2.0, 0.0, -0.2, 30.0], [0.0, 0.0, 0.1, 60.0], [2.0, 0.0, 0.0, 90.0], [4.0, 0.0, 0.3, 120.0], [0.0, 0.0, 40.0, 0.0]]\n";

#[test]
fn mesh_particles_name_their_prototype_and_a_wake_rides_its_object() {
    let scene = open("mesh-leaves", LEAVES).unwrap();
    let leaves = &scene.particles["leaves"];
    assert_eq!(leaves.mesh.as_ref().unwrap().object, "leaf");
    let body = leaves.recipe.body.unwrap();
    assert_eq!((body.mass, body.area), (0.002, 0.004));
    assert_eq!(body.ground.unwrap().height, 0.0);
    assert_eq!(leaves.recipe.placed.len(), 6);
    let wake = &scene.wakes["car"];
    assert_eq!(wake.attach.object, "car");
    assert_eq!(wake.recipe.strength, 0.7);
    assert_eq!(wake.recipe.at, [0.0, 0.2, 0.0]);
    let draws = scene.draws();
    let shaping = shaping(&scene, &draws);
    let shown: Vec<&str> = draws
        .items
        .iter()
        .zip(&shaping)
        .filter(|(_, shaped)| **shaped)
        .map(|(draw, _)| draw.object.as_str())
        .collect();
    assert!(!shown.is_empty() && shown.iter().all(|name| *name == "leaf"));
    let shape = &shapes(&scene, &draws)[0];
    let mut low = [f32::INFINITY; 3];
    let mut high = [f32::NEG_INFINITY; 3];
    for (index, local) in shape {
        for p in &draws.items[*index].geometry.positions {
            let q = pfx_core::sim::particles::point(*local, *p);
            for axis in 0..3 {
                low[axis] = low[axis].min(q[axis]);
                high[axis] = high[axis].max(q[axis]);
            }
        }
    }
    for axis in 0..3 {
        assert!((low[axis] + high[axis]).abs() < 1e-4, "{low:?} {high:?}");
        assert!(
            high[axis] - low[axis] < 0.5,
            "the prototype keeps its scale"
        );
    }
    let ghost = open(
        "mesh-ghost",
        "[[particles]]\nname = \"leaves\"\nmesh = \"ghost\"\n",
    )
    .unwrap_err();
    assert!(ghost.message.contains("ghost"), "{ghost}");
    let nodeless = open("wake-node", "[object.wake]\nnode = \"wheel_rl\"\n").unwrap_err();
    assert!(nodeless.message.contains("wheel_rl"), "{nodeless}");
}

fn passing(t: f32) -> Matrix {
    let mut m = IDENTITY;
    m[3][0] = 15.0 * t - 12.0;
    m[3][1] = 0.3;
    m
}

fn swept(seconds: f32) -> Vec<pfx_core::sim::particles::Piece> {
    let scene = open("mesh-wake", LEAVES).unwrap();
    let mut sprays = super::sprays::Sprays::of(&scene);
    sprays.seek(seconds, |t, out| {
        out.push(IDENTITY);
        out.push(passing(t));
    });
    let mut sprites = 0;
    sprays.each(|_, shown| sprites += shown.len());
    assert_eq!(sprites, 0, "mesh particles draw no sprites");
    let mut out = Vec::new();
    sprays.each_piece(|index, _, pieces| {
        assert_eq!(index, 0);
        out.extend_from_slice(pieces);
    });
    out
}

#[test]
fn a_passing_wake_lifts_resting_pieces_that_settle_again_the_same_each_time() {
    let resting = swept(0.5);
    assert_eq!(resting.len(), 6);
    assert!(resting.iter().all(|piece| piece.resting));
    let stirred = swept(1.3);
    assert!(
        stirred.iter().take(5).any(|piece| !piece.resting),
        "{stirred:?}"
    );
    let settled = swept(12.0);
    assert_eq!(settled, swept(12.0));
    for piece in &settled {
        assert!(piece.resting, "{piece:?}");
        assert_eq!(piece.position[1], 0.0);
    }
    assert!(
        settled[..5]
            .iter()
            .zip(&resting)
            .any(|(after, before)| after.position != before.position)
    );
    assert_eq!(
        settled[5], resting[5],
        "a piece far from the wake stays put"
    );
}

#[test]
fn mesh_particle_and_wake_keys_out_of_range_refuse_the_scene() {
    for bad in [
        "[[particles]]\nname = \"leaves\"\nmass = 0.0\n",
        "[[particles]]\nname = \"leaves\"\nflat = 2.0\n",
        "[[particles]]\nname = \"leaves\"\ntumble = [3.0, 1.0]\n",
        "[[particles]]\nname = \"leaves\"\nbounce = 1.5\n",
        "[[particles]]\nname = \"leaves\"\nfriction = -0.1\n",
        "[[particles]]\nname = \"leaves\"\nlift = -1.0\n",
        "[[particles]]\nname = \"leaves\"\nground = inf\n",
        "[[particles]]\nname = \"leaves\"\nplaced = [[0.0, nan, 0.0, 0.0]]\n",
        "[[particles]]\nname = \"leaves\"\nmesh_node = \"Block\"\n",
        "[[particles]]\nname = \"leaves\"\nmesh = \"car\"\nmesh_node = \"wheel_rl\"\n",
        "[object.wake]\nradius = 0.0\n",
        "[object.wake]\ndecay = -1.0\n",
        "[object.wake]\nstrength = -0.5\n",
        "[object.wake]\nat = [0.0, inf, 0.0]\n",
        "[object.wake]\ngust = 1.0\n",
    ] {
        assert!(open("mesh-refused", bad).is_err(), "{bad}");
    }
}

#[test]
fn a_mesh_node_names_one_node_of_the_prototype_and_sprites_keep_no_body() {
    let scene = open(
        "mesh-node",
        "[[particles]]\nname = \"leaves\"\nmesh = \"car\"\nmesh_node = \"Block\"\nburst = 3\n\n[[particles]]\nname = \"spray\"\nrate = 5.0\n",
    )
    .unwrap();
    let mesh = scene.particles["leaves"].mesh.as_ref().unwrap();
    assert_eq!(mesh.node.as_ref().unwrap().name, "Block");
    assert!(scene.particles["leaves"].recipe.body.is_some());
    assert_eq!(scene.particles["spray"].recipe.body, None);
    assert_eq!(scene.particles["spray"].mesh, None);
    assert!(scene.wakes.is_empty());
    let draws = scene.draws();
    let shapes = shapes(&scene, &draws);
    assert_eq!(shapes.len(), 2);
    assert!(!shapes[0].is_empty());
    assert!(shapes[1].is_empty());
    let mut sprays = super::sprays::Sprays::of(&scene);
    sprays.seek_posed(0.5, |_| None);
    let mut pieces = 0;
    sprays.each_piece(|_, _, shown| pieces += shown.len());
    let mut sprites = 0;
    sprays.each(|_, shown| sprites += shown.len());
    assert_eq!(pieces, 3);
    assert!(sprites > 0);
    assert!(sprays.meshed(0) && !sprays.meshed(1) && !sprays.meshed(7));
}
