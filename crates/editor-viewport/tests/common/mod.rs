#![allow(dead_code)]

use std::path::{Path, PathBuf};

pub const SCENE: &str = "stand.scene.toml";

pub fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tmp/viewport-tests")
        .join(name);
    if dir.exists() {
        std::fs::remove_dir_all(&dir).unwrap();
    }
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

pub fn data(folder: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(folder)
}

fn copy(from: &Path, to: &Path) {
    let mut entries: Vec<_> = std::fs::read_dir(from)
        .unwrap()
        .flatten()
        .map(|entry| entry.path())
        .collect();
    entries.sort();
    for entry in entries {
        std::fs::copy(&entry, to.join(entry.file_name().unwrap())).unwrap();
    }
}

pub fn fixture(name: &str) -> PathBuf {
    let to = scratch(name);
    copy(&data("stand"), &to);
    to.join(SCENE)
}

pub const PLACEMENT: &str = "\n[[object]]\nname = \"pair\"\nprefab = \"pair.prefab.toml\"\nat = [1.1, 0.0, -0.9]\nid = \"pa1rgr0vp5\"\n";

pub fn group(name: &str) -> PathBuf {
    let to = scratch(name);
    copy(&data("stand"), &to);
    copy(&data("group"), &to);
    let scene = to.join(SCENE);
    let text = read(&scene).replacen("\n[[light]]", &format!("{PLACEMENT}\n[[light]]"), 1);
    std::fs::write(&scene, text).unwrap();
    scene
}

pub fn format0(name: &str) -> PathBuf {
    let to = scratch(name);
    copy(&data("stand"), &to);
    copy(&data("format0"), &to);
    to.join(SCENE)
}

pub fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap()
}
