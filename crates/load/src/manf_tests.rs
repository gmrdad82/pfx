use std::path::{Path, PathBuf};

use crate::Mesh;
use crate::fixture::{head, lod_glb, manf_from_glb, manf_from_gltf, skinned_column, timeline_glb};
use crate::scene::Scene;

const TRIANGLE: &str = include_str!("../tests/data/triangle.gltf");
const TRIANGLE_BIN: &[u8] = include_bytes!("../tests/data/triangle.bin");
const SWATCH: &[u8] = include_bytes!("../tests/data/swatch8.png");

struct Folder {
    root: PathBuf,
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn folder(name: &str) -> Folder {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tmp/manf-tests")
        .join(name);
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    Folder { root }
}

fn scene(folder: &Folder, file: &str) -> Result<Scene, String> {
    let path = folder.root.join(format!("{file}.scene.toml"));
    std::fs::write(
        &path,
        format!(
            "format = 1\n\n[mesh.car]\nfile = \"{file}\"\n\n[[object]]\nname = \"car\"\nmesh = \"car\"\n"
        ),
    )
    .unwrap();
    Scene::open(&path).map_err(String::from)
}

fn textured() -> String {
    TRIANGLE.replace(
        "\"materials\":[{\"name\":\"ink\"}]",
        "\"materials\":[{\"name\":\"ink\",\"pbrMetallicRoughness\":{\"baseColorTexture\":{\"index\":0}}}],\"textures\":[{\"source\":0}],\"images\":[{\"uri\":\"maps/swatch8.png\"}]",
    )
}

#[test]
fn a_manf_written_from_a_glb_parses_equal_to_it() {
    for glb in [
        head(),
        timeline_glb(),
        skinned_column(3),
        lod_glb("ball", None, true),
    ] {
        let expected = Mesh::parse(&glb).unwrap();
        for compress in [true, false] {
            let manf = manf_from_glb(&glb, &[], compress);
            assert!(crate::is_manf(&manf));
            assert_eq!(Mesh::parse(&manf).unwrap(), expected);
        }
    }
}

#[test]
fn a_scene_opens_a_manf_mesh_like_its_glb_and_refuses_a_cut_or_damaged_one_by_name() {
    let folder = folder("scene");
    let glb = head();
    let manf = manf_from_glb(&glb, &[], true);
    std::fs::write(folder.root.join("car.glb"), &glb).unwrap();
    std::fs::write(folder.root.join("car.manf"), &manf).unwrap();
    let from_glb = scene(&folder, "car.glb").unwrap();
    let from_manf = scene(&folder, "car.manf").unwrap();
    assert_eq!(from_manf.meshes["car"].parts, from_glb.meshes["car"].parts);
    assert_eq!(from_manf.meshes["car"].rig, from_glb.meshes["car"].rig);
    assert_eq!(from_manf.animations, from_glb.animations);
    assert!(from_manf.files.contains_key(&folder.root.join("car.manf")));

    std::fs::write(folder.root.join("cut.manf"), &manf[..manf.len() - 9]).unwrap();
    let cut = scene(&folder, "cut.manf").unwrap_err();
    assert!(cut.contains("cut.manf") && cut.contains("corrupt"), "{cut}");

    let mut damaged = manf_from_glb(&glb, &[], false);
    let bin = manf::Reader::from_bytes(&damaged).unwrap().entries()[1].clone();
    assert_eq!(bin.tag, manf::GBIN);
    damaged[(bin.offset + bin.stored / 2) as usize] ^= 0xff;
    std::fs::write(folder.root.join("damaged.manf"), &damaged).unwrap();
    let damaged = scene(&folder, "damaged.manf").unwrap_err();
    assert!(
        damaged.contains("damaged.manf") && damaged.contains("checksum"),
        "{damaged}"
    );

    assert!(Mesh::parse(&manf[..20]).unwrap_err().contains("corrupt"));
}

#[test]
fn named_file_chunks_resolve_buffers_and_textures_before_the_disk() {
    let json = textured();
    let disk = |uri: &str| match uri {
        "triangle.bin" => Ok(TRIANGLE_BIN.to_vec()),
        "maps/swatch8.png" => Ok(SWATCH.to_vec()),
        other => Err(format!("no {other}")),
    };
    let expected = Mesh::parse_with(json.as_bytes(), disk).unwrap();
    assert!(expected.surfaces[0].maps.base.is_some());

    let packed = manf_from_gltf(
        json.as_bytes(),
        &[("triangle.bin", TRIANGLE_BIN), ("maps/swatch8.png", SWATCH)],
    );
    assert_eq!(Mesh::parse(&packed).unwrap(), expected);

    let beside = manf_from_gltf(json.as_bytes(), &[("triangle.bin", TRIANGLE_BIN)]);
    assert_eq!(Mesh::parse_with(&beside, disk).unwrap(), expected);
    assert!(
        Mesh::parse(&beside).unwrap().surfaces[0]
            .maps
            .base
            .is_none()
    );
}
