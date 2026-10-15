use std::path::{Path, PathBuf};

use pfx_core::anim::{Animator, Interpolation};
use serde_json::Value;

use crate::Mesh;
use crate::fixture::{HEAD_KEYS, HEAD_TARGETS, HEAD_TIMES, HEAD_VERTICES, HEAD_WEIGHTS, glb};
use crate::scene::Scene;

const HEAD: &[u8] = include_bytes!("../tests/data/head.glb");

fn split(bytes: &[u8]) -> (Value, Vec<u8>) {
    let length = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
    let json = serde_json::from_slice(&bytes[20..20 + length]).unwrap();
    let bin = bytes[20 + length + 8..].to_vec();
    (json, bin)
}

fn edited(change: impl FnOnce(&mut Value)) -> Result<Mesh, String> {
    let (mut json, bin) = split(HEAD);
    change(&mut json);
    Mesh::parse(&glb(&json.to_string(), &bin))
}

fn close(a: &[f32], b: &[f32]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-5)
}

#[test]
fn the_head_keeps_its_targets_default_weights_hat_and_weights_channel() {
    let mesh = Mesh::parse(HEAD).unwrap();
    let head = &mesh.groups[0];
    assert_eq!(head.weights, HEAD_WEIGHTS);
    assert_eq!(head.default_weights(), HEAD_WEIGHTS);
    let names: Vec<&str> = head
        .extra_array("targetNames")
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert_eq!(names, HEAD_TARGETS);
    let primitive = &head.primitives[0];
    assert_eq!(primitive.positions.len(), HEAD_VERTICES);
    assert_eq!(primitive.targets.len(), HEAD_TARGETS.len());
    for target in &primitive.targets {
        assert_eq!(target.positions.len(), HEAD_VERTICES);
        assert_eq!(target.normals.as_ref().map(Vec::len), Some(HEAD_VERTICES));
        assert_eq!(target.tangents, None);
    }
    let jaw = &primitive.targets[0];
    let lowest = (0..HEAD_VERTICES)
        .min_by(|&a, &b| jaw.positions[a][1].total_cmp(&jaw.positions[b][1]))
        .unwrap();
    assert!(jaw.positions[lowest][1] < -0.05);
    assert!(primitive.positions[lowest][1] < 0.0 && primitive.positions[lowest][2] > 0.0);
    let brow = &primitive.targets[2];
    let moved: Vec<usize> = (0..HEAD_VERTICES)
        .filter(|&at| brow.positions[at] != [0.0; 3])
        .collect();
    assert!(!moved.is_empty() && moved.len() < HEAD_VERTICES / 4);
    assert!(moved.iter().all(|&at| {
        let p = primitive.positions[at];
        p[1] > 0.0 && p[2] > 0.0 && brow.positions[at][1] > 0.0
    }));
    assert_eq!(brow.positions[0], [0.0; 3]);

    let hat = &mesh.nodes[1];
    assert_eq!(hat.name, "hat");
    assert_eq!(hat.parent, Some(0));
    assert_eq!(hat.trs.translation, [0.0, 0.55, 0.0]);
    let hat = &mesh.groups[hat.group.unwrap() as usize];
    assert!(hat.primitives[0].targets.is_empty());
    assert!(hat.weights.is_empty() && hat.default_weights().is_empty());

    let talk = &mesh.animations[0];
    assert_eq!(talk.name, "talk");
    assert!(talk.channels.is_empty());
    let channel = &talk.weights[0];
    assert_eq!(
        (channel.node, channel.targets, channel.interpolation),
        (0, 3, Interpolation::Linear)
    );
    assert_eq!(channel.times, HEAD_TIMES);
    let mut out = [0.0; 3];
    for (time, expected) in [
        (0.0, HEAD_KEYS[0]),
        (0.125, [0.5, 0.3, 0.25]),
        (0.6, [0.12, 0.3, 0.6]),
        (5.0, HEAD_KEYS[3]),
    ] {
        channel.sample(time, &mut out);
        assert!(close(&out, &expected), "{time}: {out:?}");
    }
}

#[test]
fn weights_keys_step_and_a_cubic_channel_needs_its_tangents() {
    let step = edited(|json| json["animations"][0]["samplers"][0]["interpolation"] = "STEP".into())
        .unwrap();
    let mut out = [0.0; 3];
    step.animations[0].weights[0].sample(0.49, &mut out);
    assert_eq!(out, HEAD_KEYS[1]);
    let cubic =
        edited(|json| json["animations"][0]["samplers"][0]["interpolation"] = "CUBICSPLINE".into())
            .unwrap_err();
    assert!(cubic.contains("holds 12 weights for node 0"), "{cubic}");
}

#[test]
fn a_target_shorter_than_its_primitive_is_refused() {
    let short = edited(|json| {
        let accessor = json["meshes"][0]["primitives"][0]["targets"][0]["POSITION"]
            .as_u64()
            .unwrap() as usize;
        json["accessors"][accessor]["count"] = (HEAD_VERTICES - 1).into();
    })
    .unwrap_err();
    assert_eq!(
        short,
        format!(
            "mesh: morph target 0 positions hold {} deltas for {HEAD_VERTICES} vertices",
            HEAD_VERTICES - 1
        )
    );
    let sparse = edited(|json| {
        let accessor = json["meshes"][0]["primitives"][0]["targets"][2]["NORMAL"]
            .as_u64()
            .unwrap() as usize;
        json["accessors"][accessor]["count"] = (HEAD_VERTICES + 1).into();
    })
    .unwrap_err();
    assert!(
        sparse.starts_with("mesh: morph target 2 normals hold"),
        "{sparse}"
    );
    let extra =
        edited(|json| json["meshes"][0]["weights"] = serde_json::json!([0.0, 0.0, 0.0, 1.0]))
            .unwrap_err();
    assert!(
        extra.contains("4 default weights for 3 morph targets"),
        "{extra}"
    );
}

struct Folder {
    root: PathBuf,
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn a_scene_head_carries_its_morph_set_and_the_animator_samples_its_weights() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tmp/morph-tests/head");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let folder = Folder { root };
    std::fs::write(folder.root.join("head.glb"), HEAD).unwrap();
    let path = folder.root.join("head.scene.toml");
    std::fs::write(
        &path,
        "format = 1\n\n[mesh.head]\nfile = \"head.glb\"\nid = \"mrphd00001\"\n\n\
         [[object]]\nname = \"face\"\nmesh = \"head\"\nid = \"mrpfc00001\"\n",
    )
    .unwrap();
    let scene = Scene::open(&path).unwrap();
    let mesh = &scene.meshes["head"];
    let rig = mesh.rig.as_ref().expect("a morphed mesh has a rig");
    assert_eq!(rig.morphs().len(), 1);
    assert_eq!(rig.morphs()[0].name, "head");
    assert_eq!(rig.morphs()[0].defaults, HEAD_WEIGHTS);
    let parts: Vec<(&str, Option<usize>)> = mesh
        .parts
        .iter()
        .map(|part| {
            (
                part.node.as_str(),
                part.geometry.morph.as_ref().map(|morph| morph.morph),
            )
        })
        .collect();
    assert_eq!(parts, [("head", Some(0)), ("hat", None)]);
    let morph = mesh.parts[0].geometry.morph.as_ref().unwrap();
    assert_eq!(morph.targets.len(), 3);

    let mut animator = Animator::new(rig.clone(), 60);
    assert_eq!(animator.weights(0), [HEAD_WEIGHTS.to_vec()]);
    animator.play("talk", 0).unwrap();
    assert!(close(&animator.weights(15)[0], &HEAD_KEYS[1]));
    assert!(close(&animator.weights(36)[0], &[0.12, 0.3, 0.6]));
}
