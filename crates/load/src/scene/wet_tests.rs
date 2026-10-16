use std::path::{Path, PathBuf};

use super::*;

struct Folder {
    root: PathBuf,
}

impl Folder {
    fn new(name: &str) -> Self {
        let root = std::path::absolute(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../tmp/scene-wet-tests")
                .join(name),
        )
        .unwrap();
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/scenes");
        for entry in std::fs::read_dir(fixtures).unwrap() {
            let entry = entry.unwrap();
            std::fs::copy(entry.path(), root.join(entry.file_name())).unwrap();
        }
        Self { root }
    }

    fn scene(&self) -> PathBuf {
        self.root.join("room.scene.toml")
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn a_material_wet_table_reads_and_edits_live() {
    let folder = Folder::new("edit");
    let mut edit = SceneEdit::open(folder.scene()).unwrap();
    let dry = edit.scene().library.get("clay").unwrap().wet;
    assert!(dry.inactive());
    edit.set_material("clay", &["wet", "amount"], 0.6f32)
        .unwrap();
    edit.set_material("clay", &["wet", "puddles"], 0.4f32)
        .unwrap();
    edit.set_material("clay", &["wet", "seed"], 9i64).unwrap();
    let wet = edit.scene().library.get("clay").unwrap().wet;
    assert!((wet.amount - 0.6).abs() < 1e-6);
    assert!((wet.puddles - 0.4).abs() < 1e-6);
    assert_eq!(wet.puddle_scale, pfx_materials::Wet::default().puddle_scale);
    assert_eq!(wet.seed, 9);
    assert!(
        edit.set_material("clay", &["wet", "puddles"], 1.5f32)
            .is_err()
    );
    let reopened = Scene::open(folder.scene()).unwrap();
    assert_eq!(reopened.library.get("clay").unwrap().wet, wet);
    let before = Scene::open(folder.scene()).unwrap();
    edit.set_material("clay", &["wet", "puddle_scale"], 5.0f32)
        .unwrap();
    let after = Scene::open(folder.scene()).unwrap();
    assert_eq!(before.diff(&after).materials.changed, vec!["clay"]);
}
