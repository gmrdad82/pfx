use std::path::{Path, PathBuf};

use pfx_scene::Project;

fn copied(name: &str) -> PathBuf {
    let from = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/demo");
    let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("resolve")
        .join(name);
    let _ = std::fs::remove_dir_all(&root);
    for folder in ["assets", "materials", "prefabs", "scenes"] {
        std::fs::create_dir_all(root.join(folder)).unwrap();
        for entry in std::fs::read_dir(from.join(folder)).unwrap() {
            let entry = entry.unwrap();
            std::fs::copy(entry.path(), root.join(folder).join(entry.file_name())).unwrap();
        }
    }
    std::fs::copy(from.join("project.toml"), root.join("project.toml")).unwrap();
    root
}

#[test]
fn a_path_resolves_from_its_own_folder_first_then_from_the_project_root() {
    let root = copied("near-first");
    let wing = root.join("scenes/wing");
    std::fs::create_dir_all(&wing).unwrap();
    let room = std::fs::read_to_string(root.join("scenes/room.scene.toml")).unwrap();
    let room = room
        .replace(
            "materials = [\"materials/room.materials.toml\"]",
            "materials = [\"room.materials.toml\", \"../../materials/room.materials.toml\"]",
        )
        .replace("material = \"oak\"", "material = \"pine\"")
        .replace(
            "file = \"assets/ball.gltf\"",
            "file = \"../../assets/ball.gltf\"",
        );
    std::fs::write(wing.join("room.scene.toml"), room).unwrap();
    std::fs::remove_file(root.join("scenes/room.scene.toml")).unwrap();
    let marker = std::fs::read_to_string(root.join("project.toml")).unwrap();
    std::fs::write(
        root.join("project.toml"),
        marker.replace("scenes/room.scene.toml", "scenes/wing/room.scene.toml"),
    )
    .unwrap();
    std::fs::write(
        wing.join("room.materials.toml"),
        "format = 1\n\n[materials.pine]\nid = \"p1nep1nep1\"\nbase = [0.1, 0.2, 0.3]\n",
    )
    .unwrap();
    let project = Project::open(&root).unwrap();
    let scene = project
        .scene(Path::new("scenes/wing/room.scene.toml"))
        .unwrap_or_else(|errors| panic!("{errors:?}"));
    assert!(
        scene
            .files
            .contains(&PathBuf::from("scenes/wing/room.materials.toml")),
        "{:?}",
        scene.files
    );
    let pine = scene.materials.iter().find(|m| m.key == "pine").unwrap();
    assert_eq!(pine.file, Path::new("scenes/wing/room.materials.toml"));
    let brass = scene.materials.iter().find(|m| m.key == "brass").unwrap();
    assert_eq!(brass.file, Path::new("materials/room.materials.toml"));
    let mut files: Vec<&str> = scene.meshes.iter().map(|m| m.value.file.as_str()).collect();
    files.sort_unstable();
    files.dedup();
    assert_eq!(files, ["assets/ball.gltf", "assets/block.gltf"]);
    let found = project
        .locate(
            Path::new("scenes/wing/room.scene.toml"),
            "room.materials.toml",
        )
        .unwrap();
    assert_eq!(found, "scenes/wing/room.materials.toml");
    assert_eq!(
        project
            .locate(
                Path::new("scenes/wing/room.scene.toml"),
                "prefabs/crate.prefab.toml"
            )
            .unwrap(),
        "prefabs/crate.prefab.toml"
    );
    let errors: Vec<String> = project
        .check()
        .iter()
        .filter(|d| d.severity == pfx_scene::Severity::Error)
        .map(ToString::to_string)
        .collect();
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}
