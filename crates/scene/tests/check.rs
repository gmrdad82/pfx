mod common;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use common::{Folder, LIGHTS, PROJECT, ROOM, codes, show};
use pfx_scene::types::{Scale, Trace};
use pfx_scene::{File, Project, check, code};

#[test]
fn the_room_project_checks_clean() {
    let folder = Folder::room("clean");
    let diagnostics = folder.check();
    assert!(diagnostics.is_empty(), "{}", show(&diagnostics));
}

#[test]
fn the_readmes_demo_project_checks_clean_and_resolves() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/demo");
    let project = Project::open(&root).unwrap_or_else(|diagnostic| panic!("{diagnostic}"));
    let diagnostics = project.check();
    assert!(diagnostics.is_empty(), "{}", show(&diagnostics));
    let scene = project
        .scene(Path::new("scenes/room.scene.toml"))
        .unwrap_or_else(|diagnostics| panic!("{}", show(&diagnostics)));
    assert_eq!(scene.objects.len(), 6);
}

#[test]
fn the_room_resolves_to_a_scene() {
    let folder = Folder::room("resolve");
    let scene = folder
        .project()
        .scene(&folder.path("room.scene.toml"))
        .unwrap_or_else(|diagnostics| panic!("{}", show(&diagnostics)));
    let names: Vec<&str> = scene
        .objects
        .iter()
        .map(|entry| entry.key.as_str())
        .collect();
    assert_eq!(
        names,
        ["floor", "crate", "left pillar", "right pillar", "screen"]
    );
    assert_eq!(scene.lights.len(), 2);
    assert_eq!(scene.materials.len(), 6);
    assert_eq!(scene.fallback.as_deref(), Some("grey"));
    let right = scene.object("right pillar").unwrap();
    assert_eq!(right.value.parent.as_deref(), Some("left pillar"));
    assert_eq!(right.value.materials["Pillar"], "blue");
    assert_eq!(right.id.as_deref(), Some("0bj00000p2"));
    assert_eq!(
        scene.mover("spin").unwrap().value.objects,
        ["left pillar", "crate"]
    );
    assert_eq!(
        scene.mesh("pillar").unwrap().value.nodes["Cap"]
            .material
            .as_deref(),
        Some("metal")
    );
    assert_eq!(
        scene.light("warm").unwrap().file,
        Path::new("lights.scene.toml")
    );
}

#[test]
fn the_trace_keys_reach_the_scene_and_a_value_out_of_range_is_refused_at_its_key() {
    let folder = Folder::room("trace");
    let scene = |trace: &str| {
        folder.write("room.scene.toml", &format!("{ROOM}\n[trace]\n{trace}"));
        folder.project().scene(&folder.path("room.scene.toml"))
    };

    let kept = scene("clamp_indirect = 4\nfilter_glossy = 0.25\n")
        .unwrap_or_else(|diagnostics| panic!("{}", show(&diagnostics)));
    let trace = kept.trace.unwrap();
    assert_eq!(trace.clamp_indirect, Some(4.0));
    assert_eq!(trace.filter_glossy, Some(0.25));
    assert!(!trace.transmissive_shadows);

    let edges = scene("clamp_indirect = 10000\nfilter_glossy = 1\n")
        .unwrap_or_else(|diagnostics| panic!("{}", show(&diagnostics)));
    assert_eq!(edges.trace.unwrap().clamp_indirect, Some(10000.0));

    let off = scene("transmissive_shadows = true\n")
        .unwrap_or_else(|diagnostics| panic!("{}", show(&diagnostics)));
    let trace = off.trace.unwrap();
    assert_eq!(trace.clamp_indirect, None);
    assert_eq!(trace.filter_glossy, None);
    assert_eq!(Trace::CLAMP_INDIRECT, 0.0);
    assert_eq!(Trace::FILTER_GLOSSY, 0.0);

    let refused = scene("clamp_indirect = -1\nfilter_glossy = nan\n").unwrap_err();
    let found: Vec<(&str, &str, u32, u32, &str)> = refused
        .iter()
        .map(|diagnostic| {
            (
                diagnostic.code,
                diagnostic.key.as_str(),
                diagnostic.line,
                diagnostic.column,
                diagnostic.message.as_str(),
            )
        })
        .collect();
    let line = ROOM.lines().count() as u32 + 3;
    assert_eq!(
        found,
        [
            (
                code::BAD_VALUE,
                "trace.clamp_indirect",
                line,
                18,
                "[trace] clamp_indirect = -1 is outside 0 to 10000"
            ),
            (
                code::BAD_VALUE,
                "trace.filter_glossy",
                line + 1,
                17,
                "[trace] filter_glossy = NaN is outside 0 to 1"
            ),
        ],
        "{}",
        show(&refused)
    );
}

#[test]
fn references_take_an_id_or_a_unique_name() {
    let folder = Folder::room("references");
    let text = ROOM
        .replace(
            "mesh = \"block\"\nat = [0.0, -0.05",
            "mesh = \"me5h00000b\"\nat = [0.0, -0.05",
        )
        .replace(
            "material = \"grey\"\n\n[[object]]",
            "material = \"mat0000002\"\n\n[[object]]",
        )
        .replace("parent = \"left pillar\"", "parent = \"0bj00000p1\"");
    folder.write("room.scene.toml", &text);
    let scene = folder
        .project()
        .scene(&folder.path("room.scene.toml"))
        .unwrap();
    let floor = &scene.object("floor").unwrap().value;
    assert_eq!(floor.mesh.as_deref(), Some("block"));
    assert_eq!(floor.material.as_deref(), Some("clay"));
    assert_eq!(
        scene
            .object("right pillar")
            .unwrap()
            .value
            .parent
            .as_deref(),
        Some("left pillar")
    );
}

#[test]
fn an_unsaved_buffer_is_checked_against_the_project() {
    let folder = Folder::room("buffer");
    let project = folder.project();
    let buffer = LIGHTS.replace("intensity = 3.0", "intensity = 3.0\ncolour = 1.0");
    let diagnostics = check(&buffer, &folder.path("lights.scene.toml"), &project);
    assert_eq!(diagnostics.len(), 1, "{}", show(&diagnostics));
    assert_eq!(diagnostics[0].code, code::UNKNOWN_KEY);
    assert_eq!(diagnostics[0].file, Path::new("lights.scene.toml"));
    assert_eq!((diagnostics[0].line, diagnostics[0].column), (18, 1));
    let renamed = ROOM.replace("name = \"crate\"", "name = \"box\"");
    let diagnostics = check(&renamed, Path::new("room.scene.toml"), &project);
    let codes: Vec<&str> = diagnostics.iter().map(|d| d.code).collect();
    assert_eq!(codes, ["bad-reference"], "{}", show(&diagnostics));
    assert!(diagnostics[0].message.contains("crate names no object"));
    assert!(project.check().is_empty());
    folder.write("lights.scene.toml", &buffer);
    let fixed = check(LIGHTS, &folder.path("lights.scene.toml"), &folder.project());
    assert!(fixed.is_empty(), "{}", show(&fixed));
}

#[test]
fn a_scene_resolves_and_a_project_checks_with_unsaved_texts_over_the_files() {
    let folder = Folder::room("overlay");
    let project = folder.project();
    let room = ROOM.replace(
        "include = [\"lights.scene.toml\"]",
        "include = [\"lights.scene.toml\", \"more/lamps.scene.toml\"]",
    );
    let lamps = "format = 1\n\n[[light]]\nid = \"11ght00003\"\nname = \"desk\"\nposition = [0.0, 1.0, 0.0]\nintensity = 2.0\n";
    let lights = LIGHTS.replace("intensity = 6.0", "intensity = 7.5");
    let overlay = BTreeMap::from([
        (folder.path("room.scene.toml"), room.clone()),
        (PathBuf::from("more/lamps.scene.toml"), lamps.to_string()),
        (PathBuf::from("lights.scene.toml"), lights),
    ]);
    let scene = project
        .scene_with(&folder.path("room.scene.toml"), overlay.clone())
        .unwrap_or_else(|diagnostics| panic!("{}", show(&diagnostics)));
    assert_eq!(scene.light("desk").unwrap().value.intensity, 2.0);
    assert_eq!(scene.light("warm").unwrap().value.intensity, 7.5);
    assert!(
        scene
            .files
            .contains(&PathBuf::from("more/lamps.scene.toml"))
    );
    let diagnostics = project.check_with(overlay.clone());
    assert!(diagnostics.is_empty(), "{}", show(&diagnostics));
    assert_eq!(folder.read("room.scene.toml"), ROOM);
    assert!(!folder.path("more/lamps.scene.toml").exists());
    let mut broken = overlay;
    broken.insert(
        PathBuf::from("more/lamps.scene.toml"),
        lamps.replace("name = \"desk\"", "name = \"warm\""),
    );
    let diagnostics = project
        .scene_with(Path::new("room.scene.toml"), broken.clone())
        .unwrap_err();
    assert_eq!(diagnostics.len(), 1, "{}", show(&diagnostics));
    assert_eq!(diagnostics[0].code, code::DUPLICATE_NAME);
    assert_eq!(diagnostics[0].file, Path::new("more/lamps.scene.toml"));
    let codes: Vec<&str> = project.check_with(broken).iter().map(|d| d.code).collect();
    assert_eq!(codes, [code::DUPLICATE_NAME]);
}

#[test]
fn a_file_reads_as_its_typed_data() {
    let folder = Folder::room("typed");
    let project = folder.project();
    let Ok(File::Scene(scene)) = project.read(&folder.path("room.scene.toml")) else {
        panic!("a scene file reads as a scene");
    };
    assert_eq!(scene.format, Some(1));
    assert_eq!(scene.object.len(), 5);
    assert_eq!(scene.object[1].pick, Some(7));
    assert_eq!(scene.object[1].scale, Some(Scale::Uniform(0.7)));
    let Ok(File::Materials(library)) = project.read(&folder.path("materials.toml")) else {
        panic!("a library named by a scene reads as a library");
    };
    assert_eq!(library.materials["clay"].specular, 0.02);
    assert_eq!(library.materials["screen"].content_layer.slot, 0);
    let Ok(File::Project(file)) = project.read(&folder.path("project.toml")) else {
        panic!("project.toml reads as the project file");
    };
    assert_eq!(file.project.name, "room");
    folder.write("broken.scene.toml", "format = 1\nlights = 1\n");
    let errors = folder
        .project()
        .read(&folder.path("broken.scene.toml"))
        .unwrap_err();
    assert_eq!(errors[0].code, code::UNKNOWN_KEY);
}

#[test]
fn the_root_is_the_nearest_folder_with_a_project_file() {
    let folder = Folder::room("root");
    folder.write("scenes/deep/a.scene.toml", "format = 1\n");
    assert_eq!(
        Project::root_of(&folder.path("scenes/deep/a.scene.toml")),
        folder.root
    );
    let lone = Folder::empty("root-lone");
    lone.write("a/b.scene.toml", "format = 1\n");
    assert_eq!(
        Project::root_of(&lone.path("a/b.scene.toml")),
        lone.path("a")
    );
    let project = folder.project();
    assert!(project.has_project_file());
    let files: Vec<String> = project
        .files()
        .map(|file| file.display().to_string().replace('\\', "/"))
        .collect();
    assert_eq!(
        files,
        [
            "lights.scene.toml",
            "project.toml",
            "room.scene.toml",
            "scenes/deep/a.scene.toml"
        ]
    );
}

#[test]
fn open_skips_target_tmp_dot_folders_other_projects_and_the_listed_paths() {
    let folder = Folder::room("skip");
    folder.write(
        "project.toml",
        &PROJECT.replace(
            "scene = \"room.scene.toml\"\n",
            "scene = \"room.scene.toml\"\nignore = [\"builds\", \"./vendor/old/\", \"loose.scene.toml\"]\n",
        ),
    );
    let broken = "format = 1\n[[object]\n";
    for skipped in [
        "target/debug/a.scene.toml",
        "tmp/b.scene.toml",
        "builds/c.scene.toml",
        "vendor/old/d.scene.toml",
        ".git/e.scene.toml",
        "other/f.scene.toml",
        "loose.scene.toml",
    ] {
        folder.write(skipped, broken);
    }
    folder.write(
        "other/project.toml",
        "format = 1\n\n[project]\nname = \"other\"\n",
    );
    folder.write("vendor/kept.scene.toml", "format = 1\n");
    folder.write("levels/target/kept.scene.toml", "format = 1\n");
    folder.write("levels/tmp/kept.prefab.toml", "format = 1\n");
    let project = folder.project();
    let files: Vec<String> = project
        .files()
        .map(|file| file.display().to_string().replace('\\', "/"))
        .collect();
    assert_eq!(
        files,
        [
            "levels/target/kept.scene.toml",
            "levels/tmp/kept.prefab.toml",
            "lights.scene.toml",
            "project.toml",
            "room.scene.toml",
            "vendor/kept.scene.toml"
        ]
    );
    let diagnostics = project.check();
    assert!(diagnostics.is_empty(), "{}", show(&diagnostics));
    let Ok(File::Project(read)) = project.read(Path::new("project.toml")) else {
        panic!("the project file reads");
    };
    assert_eq!(
        read.project.ignore,
        ["builds", "./vendor/old/", "loose.scene.toml"]
    );
}

#[test]
fn a_file_a_scene_names_in_a_skipped_folder_is_still_read() {
    let folder = Folder::room("skip-named");
    folder.write(
        "tmp/extra.scene.toml",
        LIGHTS.replace("11ght0000", "11ght0001").as_str(),
    );
    folder.write(
        "room.scene.toml",
        &ROOM.replace(
            "include = [\"lights.scene.toml\"]",
            "include = [\"tmp/extra.scene.toml\"]",
        ),
    );
    let scene = folder
        .project()
        .scene(&folder.path("room.scene.toml"))
        .unwrap_or_else(|diagnostics| panic!("{}", show(&diagnostics)));
    assert_eq!(
        scene.light("warm").unwrap().file,
        Path::new("tmp/extra.scene.toml")
    );
}

#[test]
fn an_ignore_entry_is_checked_as_a_path_and_may_name_no_folder_yet() {
    let folder = Folder::room("skip-paths");
    folder.write("levels/a.scene.toml", "format = 1\n");
    let project = |ignore: &str| {
        folder.write(
            "project.toml",
            &format!("format = 1\n\n[project]\nname = \"room\"\nignore = {ignore}\n"),
        );
        folder.check()
    };
    let diagnostics = project("[\"builds\", \"/out\", \"../out\", \"Levels\", \"a\\\\b\"]");
    let found: Vec<(&str, &str, u32)> = diagnostics
        .iter()
        .map(|d| (d.code, d.key.as_str(), d.column))
        .collect();
    assert_eq!(
        found,
        [
            ("bad-path", "project.ignore.1", 21),
            ("outside-root", "project.ignore.2", 29),
            ("path-case", "project.ignore.3", 39),
            ("bad-path", "project.ignore.4", 49),
        ],
        "{}",
        show(&diagnostics)
    );
    assert_eq!(
        diagnostics[2].message,
        "path Levels is found only as levels; paths keep their case"
    );
    let diagnostics = project("\"builds\"");
    assert_eq!(codes(&diagnostics), ["bad-type"], "{}", show(&diagnostics));
    assert_eq!(diagnostics[0].key, "project.ignore");
}

#[test]
fn a_proxies_file_no_scene_names_names_objects_of_the_project() {
    let folder = Folder::room("proxies");
    let cube = "kind = \"box\"\nsize = [1.0, 1.0, 1.0]\n";
    folder.write(
        "plates/room.proxies.toml",
        &format!(
            "format = 1\n\n[[proxy]]\nid = \"pr0xy00001\"\nobject = \"0bj000000c\"\n{cube}\n[[proxy]]\nid = \"pr0xy00002\"\nobject = \"floor\"\n{cube}\n[[proxy]]\nid = \"pr0xy00003\"\nobject = \"ghost\"\n{cube}\n[[proxy]]\nobject = \"floor\"\n{cube}"
        ),
    );
    let diagnostics = folder.check();
    let found: Vec<(&str, u32)> = diagnostics.iter().map(|d| (d.code, d.line)).collect();
    assert_eq!(
        found,
        [("bad-reference", 17), ("missing-id", 21)],
        "{}",
        show(&diagnostics)
    );
}

#[test]
fn the_crate_reads_no_asset_contents_but_checks_each_path() {
    let folder = Folder::room("assets");
    std::fs::write(folder.path("block.gltf"), b"not a gltf").unwrap();
    assert!(folder.check().is_empty());
    std::fs::remove_file(folder.path("screen.png")).unwrap();
    let diagnostics = folder.check();
    let codes: Vec<&str> = diagnostics.iter().map(|d| d.code).collect();
    assert_eq!(codes, ["missing-file"], "{}", show(&diagnostics));
    assert_eq!(diagnostics[0].key, "content.screen.image");
}
