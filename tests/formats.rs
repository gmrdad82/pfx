use std::path::{Path, PathBuf};
use std::process::Command;

use pfx_load::scene::Scene;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn scratch(name: &str) -> PathBuf {
    let dir = root()
        .join("tmp")
        .join(format!("formats-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn walk(dir: &Path, wanted: &dyn Fn(&str) -> bool, found: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if path.is_dir() {
            if name != "target" && name != "tmp" && !name.starts_with('.') {
                walk(&path, wanted, found);
            }
        } else if wanted(&name) {
            found.push(path);
        }
    }
}

fn tracked(wanted: &dyn Fn(&str) -> bool) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for top in ["samples", "library", "crates", "tests"] {
        walk(&root().join(top), wanted, &mut found);
    }
    found.sort();
    found
}

fn scene_file(name: &str) -> bool {
    [
        ".scene.toml",
        ".materials.toml",
        ".prefab.toml",
        ".proxies.toml",
    ]
    .iter()
    .any(|end| name.ends_with(end))
        || name == "project.toml"
}

fn format_of(text: &str) -> Option<i64> {
    text.parse::<toml::Table>()
        .ok()?
        .get("format")
        .and_then(toml::Value::as_integer)
}

fn named(path: &Path) -> String {
    path.strip_prefix(root())
        .unwrap_or(path)
        .display()
        .to_string()
}

fn pfx(args: &[&str], file: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_pfx"))
        .env("PFX_CACHE_DIR", "off")
        .args(args)
        .arg(file)
        .output()
        .unwrap()
}

#[test]
fn every_scene_file_inside_a_tracked_document_is_format_1() {
    let documents = tracked(&|name| name.ends_with(".pitofx"));
    assert!(documents.len() >= 9, "{documents:?}");
    let mut files = 0;
    for document in &documents {
        let reader =
            pfx_file::Reader::open(document).unwrap_or_else(|e| panic!("{}: {e}", named(document)));
        for (index, entry) in reader.entries().iter().enumerate() {
            let leaf = entry.name.rsplit('/').next().unwrap_or(&entry.name);
            if !scene_file(leaf) {
                continue;
            }
            let bytes = reader.read(index).unwrap();
            let text = String::from_utf8_lossy(&bytes);
            assert_eq!(
                format_of(&text),
                Some(1),
                "{} holds {} without `format = 1`",
                named(document),
                entry.name
            );
            files += 1;
        }
    }
    assert!(files > 0);
}

#[test]
fn every_tracked_document_holding_a_scene_opens_without_a_warning() {
    let mut opened = 0;
    for document in tracked(&|name| name.ends_with(".pitofx")) {
        let reader = pfx_file::Reader::open(&document).unwrap();
        let holds_scene = reader
            .entries()
            .iter()
            .any(|entry| entry.name.ends_with(".scene.toml"));
        if !holds_scene {
            continue;
        }
        let scene = Scene::open(&document).unwrap_or_else(|e| panic!("{}: {e}", named(&document)));
        assert_eq!(
            about_format(&scene),
            Vec::<String>::new(),
            "{}",
            named(&document)
        );
        opened += 1;
    }
    assert!(opened > 0);
}

#[test]
fn every_tracked_loose_scene_file_is_format_1() {
    let files = tracked(&scene_file);
    assert!(files.len() >= 20, "{files:?}");
    for file in files {
        let text = std::fs::read_to_string(&file).unwrap();
        assert_eq!(format_of(&text), Some(1), "{}", named(&file));
    }
}

#[test]
fn every_tracked_loose_scene_opens_without_a_format_finding() {
    let mut opened = 0;
    for file in tracked(&|name| name.ends_with(".scene.toml")) {
        match Scene::open(&file) {
            Ok(scene) => {
                assert_eq!(
                    about_format(&scene),
                    Vec::<String>::new(),
                    "{}",
                    named(&file)
                );
                opened += 1;
            }
            Err(error) => assert!(
                !error.message.contains("format"),
                "{}: {error}",
                named(&file)
            ),
        }
    }
    assert!(opened >= 10, "{opened}");
}

fn about_format(scene: &Scene) -> Vec<String> {
    scene
        .warnings
        .iter()
        .filter(|warning| warning.message.contains("format"))
        .map(ToString::to_string)
        .collect()
}

fn room(dir: &Path) -> PathBuf {
    for entry in std::fs::read_dir(root().join("crates/load/tests/scenes")).unwrap() {
        let entry = entry.unwrap();
        std::fs::copy(entry.path(), dir.join(entry.file_name())).unwrap();
    }
    dir.join("room.scene.toml")
}

#[test]
fn a_toml_scene_imports_into_a_document_that_opens_as_the_same_scene() {
    let dir = scratch("import");
    let toml = room(&dir);
    let out = pfx(&["scene", "migrate"], &toml);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let document = dir.join("room.pitofx");
    assert!(document.is_file());
    let from_toml = Scene::open(&toml).unwrap();
    let from_document = Scene::open(&document).unwrap();
    assert!(
        from_document.warnings.is_empty(),
        "{:?}",
        from_document.warnings
    );
    assert_eq!(from_toml.objects, from_document.objects);
    assert_eq!(from_toml.lights, from_document.lights);
    assert_eq!(from_toml.camera, from_document.camera);
    assert_eq!(
        from_toml.meshes.keys().collect::<Vec<_>>(),
        from_document.meshes.keys().collect::<Vec<_>>()
    );
    let again = pfx(&["scene", "migrate"], &toml);
    assert!(!again.status.success());
    let refused = pfx(&["scene", "migrate"], &document);
    assert!(!refused.status.success());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_scene_without_a_format_is_refused_and_the_import_writes_nothing() {
    let dir = scratch("old");
    let toml = room(&dir);
    let text = std::fs::read_to_string(&toml).unwrap();
    let old = text.strip_prefix("format = 1\n").unwrap();
    std::fs::write(&toml, old).unwrap();
    let error = Scene::open(&toml).unwrap_err();
    assert!(error.message.contains("names no format"), "{error}");
    let out = pfx(&["scene", "migrate"], &toml);
    assert!(!out.status.success());
    let said = String::from_utf8_lossy(&out.stderr);
    assert!(said.contains("names no format"), "{said}");
    assert!(!dir.join("room.pitofx").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_toml_recipe_imports_into_a_document_that_builds() {
    let dir = scratch("recipe");
    let recipe = dir.join("pair.toml");
    std::fs::write(
        &recipe,
        "recipe = 1\nname = \"pair\"\n\n[output]\nglb = \"pair.glb\"\n\n[[object]]\nname = \"left\"\nkind = \"box\"\nsize = [1.0, 1.0, 1.0]\n\n[[object]]\nname = \"right\"\nkind = \"box\"\nsize = [1.0, 2.0, 1.0]\nat = [2.0, 0.0, 0.0]\n",
    )
    .unwrap();
    let out = pfx(&["scene", "migrate"], &recipe);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let document = dir.join("pair.pitofx");
    let doc = pfx_build::Document::open(&document).unwrap();
    let (inside, _mount) = doc.mount(&document).unwrap();
    let loaded = pfx_recipe::load(&inside).unwrap();
    let names: Vec<&str> = loaded
        .recipe
        .objects
        .iter()
        .map(|o| o.name.as_str())
        .collect();
    assert_eq!(names, ["left", "right"]);
    let refused = pfx(&["build"], &recipe);
    assert!(!refused.status.success());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_dry_run_import_names_the_document_and_writes_nothing() {
    let dir = scratch("dry");
    let toml = room(&dir);
    let before = std::fs::read_dir(&dir).unwrap().count();
    let out = pfx(&["scene", "migrate", "--dry-run"], &toml);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let said = String::from_utf8_lossy(&out.stdout);
    assert!(said.contains("would import"), "{said}");
    assert!(said.contains("room.pitofx"), "{said}");
    assert!(!dir.join("room.pitofx").exists());
    let after = std::fs::read_dir(&dir).unwrap().count();
    assert_eq!(before, after);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_scene_whose_library_names_no_format_is_refused_and_the_import_writes_nothing() {
    let dir = scratch("old-library");
    let toml = room(&dir);
    let library = dir.join("materials.toml");
    let text = std::fs::read_to_string(&library).unwrap();
    std::fs::write(&library, text.strip_prefix("format = 1\n").unwrap()).unwrap();
    let out = pfx(&["scene", "migrate"], &toml);
    assert!(!out.status.success());
    let said = String::from_utf8_lossy(&out.stderr);
    assert!(said.contains("materials.toml"), "{said}");
    assert!(said.contains("names no format"), "{said}");
    assert!(!dir.join("room.pitofx").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_folder_import_takes_every_format_1_file_and_names_each_one_it_refuses() {
    let dir = scratch("folder");
    let good = dir.join("good");
    let bad = dir.join("bad");
    std::fs::create_dir_all(&good).unwrap();
    std::fs::create_dir_all(&bad).unwrap();
    room(&good);
    let old = room(&bad);
    let text = std::fs::read_to_string(&old).unwrap();
    std::fs::write(&old, text.strip_prefix("format = 1\n").unwrap()).unwrap();
    let out = pfx(&["scene", "migrate"], &dir);
    assert!(!out.status.success());
    let said = String::from_utf8_lossy(&out.stdout);
    let refused = String::from_utf8_lossy(&out.stderr);
    assert!(good.join("room.pitofx").is_file(), "{said}\n{refused}");
    assert!(!bad.join("room.pitofx").exists(), "{said}\n{refused}");
    assert!(said.contains("imported"), "{said}");
    assert!(refused.contains("names no format"), "{refused}");
    let opened = Scene::open(good.join("room.pitofx")).unwrap();
    assert!(!opened.objects.is_empty());
    std::fs::write(&old, text).unwrap();
    let again = pfx(&["scene", "migrate"], &dir);
    assert!(
        again.status.success(),
        "{}",
        String::from_utf8_lossy(&again.stderr)
    );
    let said = String::from_utf8_lossy(&again.stdout);
    assert!(said.contains("kept"), "{said}");
    assert!(bad.join("room.pitofx").is_file(), "{said}");
    let _ = std::fs::remove_dir_all(&dir);
}
