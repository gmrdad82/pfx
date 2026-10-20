use std::fs;
use std::path::{Path, PathBuf};

use pfx_build::{Document, Registry, build_with, missing_materials};
use pfx_recipe::files::Mount;
use pfx_recipe::{Loaded, Recipe, validate};
use pfx_recipe_scene::{palette, pieces};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn recipes() -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = fs::read_dir(root().join("samples"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "pitofx"))
        .collect();
    found.sort();
    found
}

fn load(path: &Path) -> Result<Loaded, pfx_recipe::Error> {
    let (recipe, mount) = opened(path);
    let loaded = pfx_recipe::load(&recipe);
    drop(mount);
    loaded
}

fn opened(path: &Path) -> (PathBuf, Mount) {
    let doc = Document::open(path).unwrap_or_else(|e| panic!("{e}"));
    doc.mount(path).unwrap_or_else(|e| panic!("{e}"))
}

fn built(path: &Path) -> (Recipe, pfx_build::Built) {
    let (recipe, _mount) = opened(path);
    let loaded = pfx_recipe::load(&recipe).unwrap_or_else(|e| panic!("{}: {e:?}", name(path)));
    let folder = recipe.parent().unwrap();
    let built = build_with(&loaded.recipe, folder, &Registry::new())
        .unwrap_or_else(|e| panic!("{}: {e}", name(path)));
    (loaded.recipe, built)
}

fn name(path: &Path) -> String {
    path.file_name().unwrap().to_string_lossy().into_owned()
}

#[test]
fn the_sample_folder_holds_the_scenes() {
    let names: Vec<String> = recipes().iter().map(|path| name(path)).collect();
    for scene in [
        "courtyard.pitofx",
        "figurine.pitofx",
        "gallery.pitofx",
        "workshop.pitofx",
    ] {
        assert!(
            names.iter().any(|found| found == scene),
            "{scene}: {names:?}"
        );
    }
}

#[test]
fn every_sample_scene_checks_clean() {
    for path in recipes() {
        let (recipe, _mount) = opened(&path);
        let loaded = pfx_recipe::load(&recipe).unwrap_or_else(|e| panic!("{}: {e:?}", name(&path)));
        let mut problems = validate(&loaded.recipe);
        problems.extend(missing_materials(
            &loaded.recipe,
            recipe.parent().unwrap(),
            &Registry::new(),
        ));
        assert!(problems.is_empty(), "{}: {problems:?}", name(&path));
    }
}

#[test]
fn every_sample_scene_builds_and_dresses_every_piece() {
    for path in recipes() {
        let (loaded, built) = built(&path);
        let recipe = built.recipe(&loaded);
        let cut = pieces(&built.parts);
        assert!(!cut.is_empty(), "{}", name(&path));
        palette(&recipe, &cut, std::iter::repeat(false))
            .unwrap_or_else(|e| panic!("{}: {e}", name(&path)));
    }
}

#[test]
fn every_sample_scene_has_its_cameras() {
    let wanted = [
        ("workshop.pitofx", &["hero", "top", "detail", "orbit"][..]),
        ("gallery.pitofx", &["flat", "angle"][..]),
        ("courtyard.pitofx", &["chase", "hero"][..]),
    ];
    for (file, cameras) in wanted {
        let loaded = load(&root().join("samples").join(file)).unwrap();
        for camera in cameras {
            assert!(
                loaded.recipe.cameras.iter().any(|c| c.name == *camera),
                "{file}: no camera {camera}"
            );
        }
    }
}

#[test]
fn every_sample_scene_uses_only_its_own_materials() {
    for path in recipes() {
        let loaded = load(&path).unwrap();
        let used: Vec<&str> = loaded
            .recipe
            .objects
            .iter()
            .flat_map(|o| o.material.iter().chain(&o.materials))
            .map(String::as_str)
            .collect();
        let generated = loaded
            .recipe
            .objects
            .iter()
            .all(|o| matches!(o.kind, pfx_recipe::Kind::Generator { .. }));
        assert!(!used.is_empty() || generated, "{}", name(&path));
        for material in used {
            assert!(
                loaded.recipe.materials.contains_key(material),
                "{}: {material} is not the scene's own",
                name(&path)
            );
        }
    }
}

#[test]
fn the_readme_commands_name_existing_flags() {
    let usage = fs::read_to_string(root().join("docs/recipes.md")).unwrap();
    let usage = usage
        .split("## Commands")
        .nth(1)
        .and_then(|rest| rest.split("```").nth(1))
        .unwrap();
    let readme = fs::read_to_string(root().join("samples/README.md")).unwrap();
    let mut fenced = false;
    let mut checked = 0;
    for line in readme.lines() {
        if line.starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if !fenced || !line.starts_with("pfx ") || line.starts_with("pfx gui ") {
            continue;
        }
        let words: Vec<&str> = line.split_whitespace().collect();
        let command = format!("pfx {} ", words[1]);
        let mine: Vec<&str> = usage
            .lines()
            .filter(|known| known.trim_start().starts_with(&command))
            .collect();
        assert!(!mine.is_empty(), "no command: {line}");
        for word in words.iter().filter(|word| word.starts_with("--")) {
            assert!(
                mine.iter().any(|known| known.contains(word)),
                "{word} is not a flag of `{}`",
                words[1]
            );
        }
        for word in words
            .iter()
            .filter(|word| word.ends_with(".toml") || word.ends_with(".pitofx"))
        {
            assert!(root().join(word).exists(), "{word} does not exist");
        }
        checked += 1;
    }
    assert!(checked >= 4, "{checked} commands checked");
}

#[test]
fn every_sample_is_a_pfx_that_prints_its_recipe_and_holds_what_it_reads() {
    let plain: Vec<String> = fs::read_dir(root().join("samples"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|file| file.ends_with(".toml"))
        .collect();
    assert!(plain.is_empty(), "plain TOML in samples/: {plain:?}");
    for path in recipes() {
        let doc = Document::open(&path).unwrap();
        let table: toml::Table = doc
            .recipe_text
            .parse()
            .unwrap_or_else(|e| panic!("{}: {e}", name(&path)));
        assert_eq!(table["recipe"].as_integer(), Some(1), "{}", name(&path));
        let stem = path.file_stem().unwrap().to_string_lossy();
        assert_eq!(doc.recipe_name, format!("{stem}.toml"));
        let away = root().join("tmp").join("samples-alone");
        fs::create_dir_all(&away).unwrap();
        let alone = away.join(path.file_name().unwrap());
        fs::copy(&path, &alone).unwrap();
        built(&alone);
    }
}
