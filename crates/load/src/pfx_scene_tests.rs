use std::path::{Path, PathBuf};

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
        .join("../../tmp/pfx-scene-tests")
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
fn a_packed_scene_opens_from_its_pfx_alone_like_its_toml() {
    let source = folder("pack-source");
    let toml = room(&source);
    let expected = Scene::open(&toml).unwrap();
    let alone = folder("pack-alone");
    let file = alone.root.join("room.pitofx");
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
    std::fs::write(alone.root.join("cut.pitofx"), cut).unwrap();
    let refused = String::from(Scene::open(alone.root.join("cut.pitofx")).unwrap_err());
    assert!(refused.contains("cut.pitofx"), "{refused}");
}

#[test]
fn a_gltf_mesh_reads_the_pfx_files_own_gltf_and_bin() {
    let source = folder("own-source");
    let toml = room(&source);
    let expected = Scene::open(&toml).unwrap();
    let glb = head();
    let split = gltf::Glb::from_slice(&glb).unwrap();
    let scene = ROOM.replace("meshes/car.glb", pfx_scene::GLTF_PATH);
    let bytes = pfx_file::Writer::new()
        .chunk(
            pfx_file::SMET,
            &b"format = 1\nentry = \"room.scene.toml\"\n"[..],
            pfx_file::Codec::Store,
        )
        .named(
            pfx_file::SCNE,
            "room.scene.toml",
            scene.into_bytes(),
            pfx_file::Codec::Auto,
        )
        .chunk(
            pfx_file::GLTF,
            split.json.into_owned(),
            pfx_file::Codec::Auto,
        )
        .chunk(
            pfx_file::GBIN,
            split.bin.unwrap().into_owned(),
            pfx_file::Codec::Auto,
        )
        .to_vec()
        .unwrap();
    let alone = folder("own-alone");
    let file = alone.root.join("room.pitofx");
    std::fs::write(&file, bytes).unwrap();
    let opened = Scene::open(&file).unwrap();
    assert_eq!(opened.meshes["car"].parts, expected.meshes["car"].parts);
}

#[test]
fn an_edit_in_place_rewrites_the_pfx_and_the_watch_reloads_it() {
    let source = folder("watch-source");
    let toml = room(&source);
    let alone = folder("watch-alone");
    let file = alone.root.join("room.pitofx");
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

const PACKED: &[u8] = include_bytes!("../tests/data/room.pitofx");

const ROOM_EXTRA: &str = "\n[mesh.tri]\nfile = \"meshes/triangle.gltf\"\n\n[[object]]\nname = \"tri\"\nmesh = \"tri\"\nat = [0.0, 0.5, 1.0]\n\n[text.sign]\ntext = \"Open\"\nfont = \"fonts/family.ttf\"\nsize = 0.2\nat = [0.0, 2.0, -1.9]\n";

fn packed_room(folder: &Folder) -> PathBuf {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    std::fs::create_dir_all(folder.root.join("fonts")).unwrap();
    for name in [
        "lights.scene.toml",
        "materials.toml",
        "block.gltf",
        "pillar.gltf",
        "panel.gltf",
        "screen.png",
    ] {
        std::fs::copy(
            manifest.join("tests/scenes").join(name),
            folder.root.join(name),
        )
        .unwrap();
    }
    for (from, to) in [
        ("triangle.gltf", "meshes/triangle.gltf"),
        ("triangle.bin", "meshes/triangle.bin"),
        ("family.ttf", "fonts/family.ttf"),
    ] {
        std::fs::copy(manifest.join("tests/data").join(from), folder.root.join(to)).unwrap();
    }
    let mut text = std::fs::read_to_string(manifest.join("tests/scenes/room.scene.toml")).unwrap();
    text.push_str(ROOM_EXTRA);
    let path = folder.root.join("room.scene.toml");
    std::fs::write(&path, text).unwrap();
    path
}

#[test]
#[ignore = "rewrites crates/load/tests/data/room.pitofx"]
fn writes_the_packed_room_fixture() {
    let source = folder("packed-room-write");
    let toml = packed_room(&source);
    let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/room.pitofx");
    pack(&toml, &out).unwrap();
}

#[test]
fn an_embedded_pitofx_loads_its_meshes_images_and_fonts_from_its_own_bytes() {
    let source = folder("packed-room-source");
    let expected = Scene::open(packed_room(&source)).unwrap();
    let scene = crate::scene::from_pfx_bytes("room.pitofx", PACKED).unwrap();
    assert_eq!(scene.path, Path::new("room.pitofx"));
    assert_eq!(
        scene.files.keys().collect::<Vec<_>>(),
        [Path::new("room.pitofx")]
    );
    assert_eq!(scene.files[Path::new("room.pitofx")], crate::sha256(PACKED));
    assert_eq!(
        scene.meshes.keys().collect::<Vec<_>>(),
        expected.meshes.keys().collect::<Vec<_>>()
    );
    for (name, mesh) in &expected.meshes {
        assert_eq!(scene.meshes[name].parts, mesh.parts, "{name}");
    }
    assert_eq!(scene.objects, expected.objects);
    assert_eq!(scene.library, expected.library);
    assert_eq!(scene.lights, expected.lights);
    assert_eq!(
        scene.contents["screen"].rgba,
        expected.contents["screen"].rgba
    );
    assert_eq!(
        scene.texts["sign"].font_bytes,
        expected.texts["sign"].font_bytes
    );
    assert_eq!(scene.texts["sign"].family, expected.texts["sign"].family);
}

#[test]
fn an_embedded_scene_never_reads_a_file_it_does_not_hold() {
    let scene = "format = 1\n\n[mesh.tri]\nfile = \"tests/data/triangle.glb\"\n\n[[object]]\nname = \"tri\"\nmesh = \"tri\"\n";
    assert!(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/data/triangle.glb")
            .is_file()
    );
    let bytes = pfx_file::Writer::new()
        .chunk(
            pfx_file::SMET,
            &b"format = 1\nentry = \"main.scene.toml\"\n"[..],
            pfx_file::Codec::Store,
        )
        .named(
            pfx_file::SCNE,
            "main.scene.toml",
            scene.as_bytes(),
            pfx_file::Codec::Auto,
        )
        .to_vec()
        .unwrap();
    let refused = String::from(Scene::from_pfx_bytes("main.pitofx", &bytes).unwrap_err());
    assert!(refused.contains("triangle.glb"), "{refused}");
}
