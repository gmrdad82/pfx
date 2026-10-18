mod common;

use std::path::Path;

use common::{Folder, ROOM};
use pfx_file::{Codec, GLTF, RCPE, Reader, SCNE, SMET, Writer};
use pfx_scene::{Project, SceneEdit, pack, unpack};

fn packed(name: &str) -> (Folder, Vec<u8>) {
    let folder = Folder::room(name);
    let project = Project::open(&folder.root).unwrap();
    let bytes = pack(&project, Path::new("room.scene.toml")).unwrap();
    assert_eq!(bytes, pack(&project, Path::new("room.scene.toml")).unwrap());
    (folder, bytes)
}

#[test]
fn a_toml_scene_packs_unpacks_and_packs_again_byte_for_byte() {
    let (folder, bytes) = packed("pfx-round");
    let reader = Reader::from_bytes(&bytes).unwrap();
    let names: Vec<_> = reader
        .chunks(SCNE)
        .unwrap()
        .into_iter()
        .map(|(n, _)| n.to_string())
        .collect();
    assert_eq!(
        names,
        ["room.scene.toml", "lights.scene.toml", "materials.toml"]
    );
    let meta = String::from_utf8(reader.chunk(SMET).unwrap().unwrap().into_owned()).unwrap();
    assert!(meta.starts_with("format = 1\nentry = \"room.scene.toml\"\nscene = \""));
    assert!(meta.contains(&pfx_file::sha256::hex(
        &pfx_file::scene_hash(&reader).unwrap()
    )));

    let out = Folder::empty("pfx-round-out");
    let written = unpack(&bytes, &out.root).unwrap();
    assert_eq!(written.len(), 3 + 4);
    for file in &written {
        let name = file.strip_prefix(&out.root).unwrap();
        assert_eq!(
            std::fs::read(file).unwrap(),
            std::fs::read(folder.root.join(name)).unwrap(),
            "{}",
            name.display()
        );
    }
    let again = Project::open(&out.root).unwrap();
    assert_eq!(pack(&again, Path::new("room.scene.toml")).unwrap(), bytes);
}

#[test]
fn a_pfx_scene_resolves_checks_and_edits_in_place_keeping_other_chunks() {
    let (folder, bytes) = packed("pfx-edit-src");
    let original = Reader::from_bytes(&bytes).unwrap();
    let mut writer =
        Writer::new().chunk(RCPE, b"[recipe]\nkind = \"room\"\n".as_slice(), Codec::Lz4);
    for index in 0..original.entries().len() {
        writer = writer.copy(&original, index).unwrap();
    }
    let game = Folder::empty("pfx-edit");
    let file = game.root.join("room.pitofx");
    writer.write(&file).unwrap();
    let before = std::fs::read(&file).unwrap();

    let project = Project::open_pfx(&file).unwrap();
    let scene = project.scene(&file).unwrap();
    let disk = Project::open(&folder.root)
        .unwrap()
        .scene(Path::new("room.scene.toml"))
        .unwrap();
    assert_eq!(scene.objects.len(), disk.objects.len());
    assert_eq!(
        project.asset("screen.png").unwrap().as_ref(),
        std::fs::read(folder.root.join("screen.png")).unwrap()
    );
    assert!(
        Project::open(&game.root)
            .unwrap()
            .check()
            .iter()
            .all(|d| !d.is_error())
    );

    let mut edit = SceneEdit::open(&file).unwrap();
    assert_eq!(edit.path(), file.join("room.scene.toml"));
    edit.set_at("crate", [2.0, 0.35, 0.0]).unwrap();
    let after = std::fs::read(&file).unwrap();
    let reader = Reader::from_bytes(&after).unwrap();
    assert_eq!(reader.entries()[0].tag, RCPE);
    assert_eq!(reader.stored(0).unwrap(), original_rcpe(&before));
    assert_eq!(
        reader.chunk(SMET).unwrap(),
        Reader::from_bytes(&before).unwrap().chunk(SMET).unwrap()
    );
    let room = String::from_utf8(
        reader
            .named(SCNE, "room.scene.toml")
            .unwrap()
            .unwrap()
            .into_owned(),
    )
    .unwrap();
    assert_eq!(room, ROOM.replace("at = [ -0.9,", "at = [ 2.0,"));
    let reopened = Project::open_pfx(&file).unwrap().scene(&file).unwrap();
    assert_eq!(
        reopened
            .objects
            .iter()
            .find(|o| o.key == "crate")
            .unwrap()
            .value
            .at,
        [2.0, 0.35, 0.0]
    );

    edit.undo().unwrap();
    assert_eq!(std::fs::read(&file).unwrap(), before);
}

fn original_rcpe(bytes: &[u8]) -> Vec<u8> {
    Reader::from_bytes(bytes)
        .unwrap()
        .stored(0)
        .unwrap()
        .into_owned()
}

#[test]
fn broken_pfx_scenes_are_refused() {
    let game = Folder::empty("pfx-broken");
    let cases: [(&str, Vec<u8>); 4] = [
        (
            "no-meta.pitofx",
            Writer::new()
                .named(
                    SCNE,
                    "a.scene.toml",
                    b"format = 1\n".as_slice(),
                    Codec::Store,
                )
                .to_vec()
                .unwrap(),
        ),
        (
            "newer.pitofx",
            Writer::new()
                .chunk(
                    SMET,
                    b"format = 9\nentry = \"a.scene.toml\"\n".as_slice(),
                    Codec::Store,
                )
                .named(
                    SCNE,
                    "a.scene.toml",
                    b"format = 1\n".as_slice(),
                    Codec::Store,
                )
                .to_vec()
                .unwrap(),
        ),
        (
            "no-entry.pitofx",
            Writer::new()
                .chunk(
                    SMET,
                    b"format = 1\nentry = \"b.scene.toml\"\n".as_slice(),
                    Codec::Store,
                )
                .named(
                    SCNE,
                    "a.scene.toml",
                    b"format = 1\n".as_slice(),
                    Codec::Store,
                )
                .to_vec()
                .unwrap(),
        ),
        ("cut.pitofx", b"PFXFILE\0\x01\0\0\0".to_vec()),
    ];
    for (name, bytes) in cases {
        let file = game.root.join(name);
        std::fs::write(&file, bytes).unwrap();
        assert!(Project::open_pfx(&file).is_err(), "{name}");
        assert!(SceneEdit::open(&file).is_err(), "{name}");
    }
    assert_eq!(Project::open(&game.root).unwrap().check().len(), 4);
    let gltf = Writer::new()
        .chunk(
            SMET,
            b"format = 1\nentry = \"a.scene.toml\"\n".as_slice(),
            Codec::Store,
        )
        .named(
            SCNE,
            "a.scene.toml",
            b"format = 1\n\n[mesh.car]\nfile = \"@gltf\"\nnode = \"Body\"\n".as_slice(),
            Codec::Store,
        );
    let without = game.root.join("sub/no-gltf.pitofx");
    std::fs::create_dir_all(without.parent().unwrap()).unwrap();
    gltf.write(&without).unwrap();
    let found = Project::open_pfx(&without).unwrap().check();
    assert!(
        found
            .iter()
            .any(|d| d.code == "missing-file" && d.file == Path::new("no-gltf.pitofx/a.scene.toml")),
        "{found:?}"
    );
    let with = game.root.join("sub/gltf.pitofx");
    gltf.chunk(GLTF, b"{}".as_slice(), Codec::Store)
        .write(&with)
        .unwrap();
    assert!(
        Project::open_pfx(&with)
            .unwrap()
            .check()
            .iter()
            .all(|d| !d.is_error())
    );
    let out = Folder::empty("pfx-broken-out");
    std::fs::create_dir_all(out.root.join("real")).unwrap();
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(out.root.join("real"), out.root.join("assets")).unwrap();
        let linked = Writer::new()
            .named(
                pfx_file::FILE,
                "assets/x.png",
                b"x".as_slice(),
                Codec::Store,
            )
            .to_vec()
            .unwrap();
        assert!(unpack(&linked, &out.root).is_err());
        assert!(!out.root.join("real/x.png").exists());
    }
}

#[test]
fn a_recipe_document_beside_a_scene_is_not_checked_as_one_of_its_scenes() {
    let folder = Folder::room("pfx-recipe-beside");
    Writer::new()
        .chunk(
            RCPE,
            b"recipe = 1\nname = \"model\"\n".as_slice(),
            Codec::Lz4,
        )
        .write(folder.root.join("model.pfx"))
        .unwrap();
    let project = Project::open(&folder.root).unwrap();
    let named: Vec<_> = project
        .check()
        .into_iter()
        .filter(|diagnostic| diagnostic.file.ends_with("model.pfx"))
        .map(|diagnostic| diagnostic.message)
        .collect();
    assert!(named.is_empty(), "{named:?}");
}
