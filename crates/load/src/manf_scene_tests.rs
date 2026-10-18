use std::path::{Path, PathBuf};

use pfx_scene::manf;

use crate::fixture::head;
use crate::scene::{Scene, SceneEdit, SceneWatch, pack, unpack};

const ROOM: &str = "format = 1\n\n[mesh.car]\nfile = \"meshes/car.glb\"\n\n[[object]]\nname = \"car\"\nmesh = \"car\"\nat = [0.0, 0.0, 0.0]\n";

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
        .join("../../tmp/manf-scene-tests")
        .join(name);
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("meshes")).unwrap();
    Folder { root }
}

fn room(folder: &Folder) -> PathBuf {
    std::fs::write(folder.root.join("meshes/car.glb"), head()).unwrap();
    let path = folder.root.join("room.scene.toml");
    std::fs::write(&path, ROOM).unwrap();
    path
}

#[test]
fn a_packed_scene_opens_from_its_manf_alone_like_its_toml() {
    let source = folder("pack-source");
    let toml = room(&source);
    let expected = Scene::open(&toml).unwrap();
    let alone = folder("pack-alone");
    let file = alone.root.join("room.manf");
    pack(&toml, &file).unwrap();
    let opened = Scene::open(&file).unwrap();
    assert_eq!(opened.meshes["car"].parts, expected.meshes["car"].parts);
    assert_eq!(opened.objects, expected.objects);
    assert_eq!(opened.library, expected.library);
    assert_eq!(opened.files.keys().collect::<Vec<_>>(), [&file]);

    let out = folder("pack-unpacked");
    let written = unpack(&file, &out.root).unwrap();
    assert!(written.iter().any(|path| path.ends_with("meshes/car.glb")));
    let again = Scene::open(out.root.join("room.scene.toml")).unwrap();
    assert_eq!(again.meshes["car"].parts, expected.meshes["car"].parts);

    let mut cut = std::fs::read(&file).unwrap();
    cut.truncate(cut.len() - 9);
    std::fs::write(alone.root.join("cut.manf"), cut).unwrap();
    let refused = String::from(Scene::open(alone.root.join("cut.manf")).unwrap_err());
    assert!(refused.contains("cut.manf"), "{refused}");
}

#[test]
fn a_gltf_mesh_reads_the_manfs_own_gltf_and_bin() {
    let source = folder("own-source");
    let toml = room(&source);
    let expected = Scene::open(&toml).unwrap();
    let glb = head();
    let split = gltf::Glb::from_slice(&glb).unwrap();
    let scene = ROOM.replace("meshes/car.glb", pfx_scene::GLTF_PATH);
    let bytes = manf::Writer::new()
        .chunk(
            manf::SMET,
            &b"format = 1\nentry = \"room.scene.toml\"\n"[..],
            manf::Codec::Store,
        )
        .named(
            manf::SCNE,
            "room.scene.toml",
            scene.into_bytes(),
            manf::Codec::Auto,
        )
        .chunk(manf::GLTF, split.json.into_owned(), manf::Codec::Auto)
        .chunk(
            manf::GBIN,
            split.bin.unwrap().into_owned(),
            manf::Codec::Auto,
        )
        .to_vec()
        .unwrap();
    let alone = folder("own-alone");
    let file = alone.root.join("room.manf");
    std::fs::write(&file, bytes).unwrap();
    let opened = Scene::open(&file).unwrap();
    assert_eq!(opened.meshes["car"].parts, expected.meshes["car"].parts);
}

#[test]
fn an_edit_in_place_rewrites_the_manf_and_the_watch_reloads_it() {
    let source = folder("watch-source");
    let toml = room(&source);
    let alone = folder("watch-alone");
    let file = alone.root.join("room.manf");
    pack(&toml, &file).unwrap();
    let mut watch = SceneWatch::open(&file).unwrap();
    assert!(watch.check().is_none());
    let mut edit = SceneEdit::open(&file).unwrap();
    edit.set_at("car", [1.0, 0.0, 0.0]).unwrap();
    let reload = watch.check().unwrap().unwrap();
    assert_eq!(reload.diff.objects.changed, ["car"]);
    assert!(reload.diff.meshes.is_empty());
    assert_eq!(Scene::open(&file).unwrap().objects[0].at, [1.0, 0.0, 0.0]);
}
