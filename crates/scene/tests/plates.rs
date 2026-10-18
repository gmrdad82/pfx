mod common;

use std::path::Path;

use common::{Folder, LAMP, LIGHTS, ROOM, show};
use pfx_scene::types::{Casters, Plates, ProxyKind};
use pfx_scene::{Diagnostic, File, FileKind, Id, Kind, SceneEdit, Target, check, code, migrate};

const MORE: &str = "\n[[object]]\nid = \"card000001\"\nname = \"card\"\nmesh = \"panel\"\nface_camera = true\n\n[[object]]\nid = \"s10t000001\"\nname = \"slot\"\nmesh = \"block\"\ndynamic = true\n\n[[object]]\nid = \"kn0b000001\"\nname = \"knob\"\nmesh = \"block\"\nparent = \"crate\"\n\n[[object]]\nid = \"p1nned0001\"\nname = \"pinned\"\nmesh = \"block\"\nparent = \"crate\"\ndynamic = false\n";

const PLATES: &str =
    "dir = \"plates/main\"\nproxies = \"plates/room.proxies.toml\"\ncasters = \"proxies\"\n";

const PROXIES: &str = "format = 1\n\n[[proxy]]\nid = \"pr0xy00001\"\nobject = \"floor\"\nkind = \"box\"\nat = [0.0, -0.05, 0.0]\nsize = [6.0, 0.1, 6.0]\n\n[[proxy]]\nid = \"pr0xy00002\"\nobject = \"0bj00000s1\"\nkind = \"hull\"\npoints = [[0.0, 1.0, -1.9], [1.0, 1.0, -1.9], [0.0, 2.0, -1.9], [0.0, 1.0, -2.0]]\ntriangles = [[0, 2, 1], [0, 1, 3], [0, 3, 2], [1, 2, 3]]\nauthoring = { from = \"bake\" }\n";

const SIGN: &str = "\n[text.sign]\nid = \"s1gn000001\"\ntext = \"A\"\nfont = \"font.ttf\"\nsize = 0.1\ndynamic = true\n";

fn plated(name: &str, extra: &str, plates: &str, proxies: &str) -> Folder {
    let folder = Folder::room(name);
    folder.write(
        "room.scene.toml",
        &format!("{ROOM}{MORE}{extra}\n[plates]\n{plates}"),
    );
    folder.write("plates/room.proxies.toml", proxies);
    folder.write("font.ttf", "not a font");
    folder
}

fn found(diagnostics: &[Diagnostic]) -> Vec<(&'static str, &Path, &str)> {
    diagnostics
        .iter()
        .map(|d| (d.code, d.file.as_path(), d.key.as_str()))
        .collect()
}

#[test]
fn dynamic_is_derived_in_the_resolved_scene_and_kept_as_written_in_the_file() {
    let folder = plated("plates-dynamic", "", PLATES, PROXIES);
    let project = folder.project();
    let scene = project
        .scene(&folder.path("room.scene.toml"))
        .unwrap_or_else(|diagnostics| panic!("{}", show(&diagnostics)));
    let dynamic: Vec<(&str, Option<bool>)> = scene
        .objects
        .iter()
        .map(|entry| (entry.key.as_str(), entry.value.dynamic))
        .collect();
    assert_eq!(
        dynamic,
        [
            ("floor", Some(false)),
            ("crate", Some(true)),
            ("left pillar", Some(true)),
            ("right pillar", Some(true)),
            ("screen", Some(false)),
            ("card", Some(true)),
            ("slot", Some(true)),
            ("knob", Some(true)),
            ("pinned", Some(false)),
        ]
    );
    let Ok(File::Scene(file)) = project.read(&folder.path("room.scene.toml")) else {
        panic!("the room reads");
    };
    let written: Vec<Option<bool>> = file.object.iter().map(|object| object.dynamic).collect();
    assert_eq!(
        written,
        [
            None,
            None,
            None,
            None,
            None,
            None,
            Some(true),
            None,
            Some(false)
        ]
    );
}

#[test]
fn plates_name_a_folder_proxies_and_casters_and_the_scene_carries_the_proxies() {
    let folder = plated("plates-clean", SIGN, PLATES, PROXIES);
    let diagnostics = folder.check();
    assert!(diagnostics.is_empty(), "{}", show(&diagnostics));
    let project = folder.project();
    let scene = project.scene(&folder.path("room.scene.toml")).unwrap();
    assert_eq!(
        scene.plates,
        Some(Plates {
            dir: "plates/main".into(),
            proxies: Some("plates/room.proxies.toml".into()),
            casters: Some(Casters::Proxies),
        })
    );
    let proxies: Vec<(&str, Option<&str>, &str, ProxyKind)> = scene
        .proxies
        .iter()
        .map(|entry| {
            (
                entry.key.as_str(),
                entry.id.as_deref(),
                entry.value.object.as_str(),
                entry.value.kind,
            )
        })
        .collect();
    assert_eq!(
        proxies,
        [
            ("0", Some("pr0xy00001"), "floor", ProxyKind::Box),
            ("1", Some("pr0xy00002"), "screen", ProxyKind::Hull),
        ]
    );
    assert_eq!(scene.proxies[0].value.size, Some([6.0, 0.1, 6.0]));
    assert_eq!(scene.proxies[1].value.triangles.as_ref().unwrap().len(), 4);
    assert!(
        scene
            .files
            .contains(&Path::new("plates/room.proxies.toml").to_path_buf())
    );
    assert!(scene.text("sign").unwrap().value.dynamic);
    let Ok(File::Proxies(file)) = project.read(&folder.path("plates/room.proxies.toml")) else {
        panic!("the proxies file reads");
    };
    assert_eq!(file.proxy[1].object, "0bj00000s1");
    assert!(file.proxy[1].authoring.is_some());
    let text = toml::to_string(&file).unwrap();
    assert_eq!(
        toml::from_str::<pfx_scene::ProxiesFile>(&text).unwrap(),
        file
    );
}

#[test]
fn a_proxy_stands_in_for_a_static_object_of_the_scene_that_names_its_file() {
    let cases = [
        ("crate", "mover spin moves it"),
        ("right pillar", "its parent left pillar is dynamic"),
        ("card", "it faces the camera"),
        ("s10t000001", "it sets dynamic = true"),
        ("knob", "its parent crate is dynamic"),
        ("ghost", "names no object of scene room.scene.toml"),
    ];
    for (object, message) in cases {
        let proxies = format!(
            "format = 1\n\n[[proxy]]\nid = \"pr0xy00009\"\nobject = \"{object}\"\nkind = \"box\"\nsize = [1.0, 1.0, 1.0]\n"
        );
        let folder = plated("plates-static", "", PLATES, &proxies);
        let diagnostics = folder.check();
        assert_eq!(
            found(&diagnostics),
            [(
                code::BAD_REFERENCE,
                Path::new("plates/room.proxies.toml"),
                "proxy.pr0xy00009.object"
            )],
            "{}",
            show(&diagnostics)
        );
        assert!(diagnostics[0].message.contains(message), "{diagnostics:?}");
        assert_eq!(diagnostics[0].line, 5);
        if object != "ghost" {
            assert_eq!(diagnostics[0].related.len(), 1);
            assert_eq!(diagnostics[0].related[0].file, Path::new("room.scene.toml"));
        }
    }
    let proxies = "format = 1\n\n[[proxy]]\nid = \"pr0xy00009\"\nobject = \"pinned\"\nkind = \"box\"\nsize = [1.0, 1.0, 1.0]\n";
    let folder = plated("plates-pinned", "", PLATES, proxies);
    let diagnostics = folder.check();
    assert!(diagnostics.is_empty(), "{}", show(&diagnostics));
}

#[test]
fn a_proxy_reaches_a_placed_object_by_its_namespaced_id_or_key_and_never_a_placement() {
    let placement = "\n[[object]]\nid = \"1eft1amp00\"\nname = \"left lamp\"\nprefab = \"props/lamp.prefab.toml\"\n";
    let proxies = "format = 1\n\n[[proxy]]\nid = \"pr0xy00001\"\nobject = \"1eft1amp00/1amp000001\"\nkind = \"box\"\nsize = [1.0, 1.0, 1.0]\n\n[[proxy]]\nid = \"pr0xy00002\"\nobject = \"left lamp/stem\"\nkind = \"box\"\nsize = [1.0, 1.0, 1.0]\n\n[[proxy]]\nid = \"pr0xy00003\"\nobject = \"left lamp\"\nkind = \"box\"\nsize = [1.0, 1.0, 1.0]\n";
    let folder = plated("plates-placed", placement, PLATES, proxies);
    folder.write("props/lamp.prefab.toml", LAMP);
    let diagnostics = folder.check();
    assert_eq!(
        found(&diagnostics),
        [(
            code::BAD_REFERENCE,
            Path::new("plates/room.proxies.toml"),
            "proxy.pr0xy00003.object"
        )],
        "{}",
        show(&diagnostics)
    );
    assert!(diagnostics[0].message.contains("placement left lamp"));
    let fixed = proxies.replace("object = \"left lamp\"\n", "object = \"floor\"\n");
    folder.write("plates/room.proxies.toml", &fixed);
    let scene = folder
        .project()
        .scene(&folder.path("room.scene.toml"))
        .unwrap_or_else(|diagnostics| panic!("{}", show(&diagnostics)));
    let objects: Vec<&str> = scene
        .proxies
        .iter()
        .map(|entry| entry.value.object.as_str())
        .collect();
    assert_eq!(objects, ["left lamp/shade", "left lamp/stem", "floor"]);
}

#[test]
fn proxy_shapes_take_their_own_keys_with_their_ranges() {
    let cases: [(&str, &str, &str); 10] = [
        (
            "kind = \"box\"\nsize = [1.0, 1.0, 1.0]\npoints = [[0.0, 0.0, 0.0]]\n",
            code::BAD_VALUE,
            "proxy.pr0xy00009.points",
        ),
        (
            "kind = \"box\"\nat = [0.0, 1.0, 0.0]\n",
            code::MISSING_KEY,
            "proxy.pr0xy00009",
        ),
        (
            "kind = \"box\"\nsize = [1.0, 0.0, 1.0]\n",
            code::BAD_VALUE,
            "proxy.pr0xy00009.size",
        ),
        (
            "kind = \"box\"\nsize = [1.0, 1.0, 1.0]\nrotate = [nan, 0.0, 0.0]\n",
            code::BAD_VALUE,
            "proxy.pr0xy00009",
        ),
        (
            "kind = \"hull\"\nsize = [1.0, 1.0, 1.0]\npoints = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]\ntriangles = [[0, 2, 1], [0, 1, 3], [0, 3, 2], [1, 2, 3]]\n",
            code::BAD_VALUE,
            "proxy.pr0xy00009.size",
        ),
        (
            "kind = \"hull\"\npoints = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]\ntriangles = [[0, 2, 1], [0, 1, 2], [0, 2, 1], [1, 2, 0]]\n",
            code::BAD_VALUE,
            "proxy.pr0xy00009.points",
        ),
        (
            "kind = \"hull\"\npoints = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]\ntriangles = [[0, 2, 1], [0, 1, 4], [0, 3, 2], [1, 2, 3]]\n",
            code::BAD_VALUE,
            "proxy.pr0xy00009.triangles.1",
        ),
        (
            "kind = \"hull\"\npoints = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]\ntriangles = [[0, 2, 1], [0, 1, 3], [0, 3, 2]]\n",
            code::BAD_VALUE,
            "proxy.pr0xy00009.triangles",
        ),
        (
            "kind = \"hull\"\npoints = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]\n",
            code::MISSING_KEY,
            "proxy.pr0xy00009",
        ),
        (
            "kind = \"ball\"\nsize = [1.0, 1.0, 1.0]\n",
            code::BAD_VALUE,
            "proxy.pr0xy00009.kind",
        ),
    ];
    for (shape, code, key) in cases {
        let proxies =
            format!("format = 1\n\n[[proxy]]\nid = \"pr0xy00009\"\nobject = \"floor\"\n{shape}");
        let folder = plated("plates-shapes", "", PLATES, &proxies);
        let diagnostics = folder.check();
        assert_eq!(
            found(&diagnostics),
            [(code, Path::new("plates/room.proxies.toml"), key)],
            "{shape}\n{}",
            show(&diagnostics)
        );
    }
}

#[test]
fn plates_keys_are_checked_where_they_are_written() {
    let cases: [(&str, &str, &str); 8] = [
        (
            "dir = \"plates/main\"\ncasters = \"proxies\"\n",
            code::BAD_VALUE,
            "plates.casters",
        ),
        (
            "dir = \"plates/main\"\ncasters = \"all\"\n",
            code::BAD_VALUE,
            "plates.casters",
        ),
        (
            "dir = \"plates/main\"\nshadows = true\n",
            code::UNKNOWN_KEY,
            "plates.shadows",
        ),
        ("casters = \"none\"\n", code::MISSING_KEY, "plates"),
        ("dir = \"../out\"\n", code::OUTSIDE_ROOT, "plates.dir"),
        ("dir = \"PLATES/main\"\n", code::PATH_CASE, "plates.dir"),
        (
            "dir = \"plates/main\"\nproxies = \"plates/none.proxies.toml\"\n",
            code::MISSING_FILE,
            "plates.proxies",
        ),
        (
            "dir = \"plates/main\"\nproxies = \"plates/room.toml\"\n",
            code::BAD_PATH,
            "plates.proxies",
        ),
    ];
    for (plates, code, key) in cases {
        let folder = plated("plates-keys", "", plates, PROXIES);
        folder.write("plates/room.toml", "");
        folder.write("plates/main/manifest.json", "{}");
        let diagnostics = folder.check();
        assert_eq!(
            found(&diagnostics),
            [(code, Path::new("room.scene.toml"), key)],
            "{plates}\n{}",
            show(&diagnostics)
        );
    }
    let folder = plated("plates-twice", "", PLATES, PROXIES);
    folder.write(
        "lights.scene.toml",
        &format!("{LIGHTS}\n[plates]\ndir = \"plates/other\"\n"),
    );
    let diagnostics = folder.check();
    assert_eq!(
        found(&diagnostics),
        [(code::HELD_TWICE, Path::new("room.scene.toml"), "plates")],
        "{}",
        show(&diagnostics)
    );
}

#[test]
fn an_unsaved_proxies_buffer_is_checked_against_its_scene() {
    let folder = plated("plates-buffer", "", PLATES, PROXIES);
    let project = folder.project();
    let buffer = PROXIES.replace("object = \"floor\"", "object = \"crate\"");
    let marks = check(&buffer, &folder.path("plates/room.proxies.toml"), &project);
    assert_eq!(
        found(&marks),
        [(
            code::BAD_REFERENCE,
            Path::new("plates/room.proxies.toml"),
            "proxy.pr0xy00001.object"
        )],
        "{}",
        show(&marks)
    );
}

#[test]
fn a_format_0_plates_table_migrates_its_paths_to_the_root() {
    let text = "[plates]\ndir = \"plates/main\"\nproxies = \"room.proxies.toml\"\n";
    let migrated = migrate(
        text,
        Path::new("scenes/room.scene.toml"),
        Path::new("/project"),
        FileKind::Scene,
    )
    .unwrap();
    assert_eq!(
        migrated,
        "format = 1\n\n[plates]\ndir = \"scenes/plates/main\"\nproxies = \"scenes/room.proxies.toml\"\n"
    );
}

#[test]
fn the_writer_sets_plates_and_dynamic_and_refuses_casters_without_proxies() {
    let folder = Folder::room("plates-edit");
    folder.write("font.ttf", "not a font");
    let mut edit = SceneEdit::open(folder.path("room.scene.toml")).unwrap();
    edit.set(&Target::Plates, &["dir"], "plates/main").unwrap();
    edit.set(&Target::Plates, &["casters"], "none").unwrap();
    edit.set(&Target::Object("floor".into()), &["dynamic"], false)
        .unwrap();
    let before = folder.read("room.scene.toml");
    assert!(
        before.ends_with("\n[plates]\ndir = \"plates/main\"\ncasters = \"none\"\n"),
        "{before}"
    );
    let refused = edit
        .set(&Target::Plates, &["casters"], "proxies")
        .unwrap_err();
    assert_eq!(refused.code, code::BAD_VALUE);
    assert_eq!(folder.read("room.scene.toml"), before);
    edit.add(
        Kind::Text,
        Id::from_bits(7),
        "sign",
        &[
            ("dynamic", true.into()),
            ("size", 0.1f32.into()),
            ("font", "font.ttf".into()),
            ("text", "A".into()),
        ],
        None,
    )
    .unwrap();
    let after = folder.read("room.scene.toml");
    assert!(
        after.contains("text = \"A\"\nfont = \"font.ttf\"\nsize = 0.1\ndynamic = true\n"),
        "{after}"
    );
    let scene = edit.scene();
    assert_eq!(scene.object("floor").unwrap().value.dynamic, Some(false));
    assert!(scene.text("sign").unwrap().value.dynamic);
}
