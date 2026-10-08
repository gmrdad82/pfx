use std::path::PathBuf;

use pfx_editor_doc::toml_edit::value;
use pfx_editor_doc::{Disk, Files, History, Seen, Stamp, TomlPatch, parse_path};

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tmp/doc-tests")
        .join(name);
    if dir.exists() {
        std::fs::remove_dir_all(&dir).unwrap();
    }
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn a_save_stamp_tells_an_own_write_from_an_outside_one() {
    let dir = scratch("stamps");
    let path = dir.join("scene.toml");
    std::fs::write(&path, "a = 1 # one\n").unwrap();
    let mut files = Files::new();
    files.load(&path).unwrap();
    assert_eq!(files.disk(&path).unwrap(), Disk::Same);

    let mut history = History::default();
    history
        .apply(
            &mut files,
            TomlPatch::set(&path, parse_path("a").unwrap(), value(2)),
        )
        .unwrap();
    assert!(files.get(&path).unwrap().dirty());
    let first = files.save(&path).unwrap();
    assert_eq!(first, Stamp::of("a = 2 # one\n"));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "a = 2 # one\n");
    assert!(!files.get(&path).unwrap().dirty());
    assert_eq!(files.disk(&path).unwrap(), Disk::Same);
    assert_eq!(files.seen(&path, first), Some(Seen::Current));

    history.undo(&mut files).unwrap();
    let second = files.save(&path).unwrap();
    assert_eq!(files.seen(&path, second), Some(Seen::Current));
    assert_eq!(files.seen(&path, first), Some(Seen::Earlier));
    std::fs::write(&path, "a = 2 # one\n").unwrap();
    assert_eq!(files.disk(&path).unwrap(), Disk::Own);

    std::fs::write(&path, "a = 9\n").unwrap();
    assert_eq!(files.disk(&path).unwrap(), Disk::Outside("a = 9\n".into()));
    assert_eq!(files.seen(&path, Stamp::of("a = 9\n")), Some(Seen::Outside));
    let generation = files.generation();
    assert!(files.reload(&path, "a = 9\n").unwrap());
    assert!(files.generation() > generation);
    assert_eq!(files.disk(&path).unwrap(), Disk::Same);

    std::fs::remove_file(&path).unwrap();
    assert_eq!(files.disk(&path).unwrap(), Disk::Missing);
    assert!(
        std::fs::read_dir(&dir).unwrap().next().is_none(),
        "no temporary file is left"
    );
}

#[test]
fn a_failed_save_reports_and_leaves_the_file_dirty() {
    let dir = scratch("failed");
    let path = dir.join("gone").join("scene.toml");
    let mut files = Files::new();
    files.open(&path, "a = 1\n");
    files.set_text(&path, "a = 2\n").unwrap();
    let error = files.save(&path).unwrap_err();
    assert_eq!(error.file, path);
    assert!(files.get(&path).unwrap().dirty());
    assert!(files.load(dir.join("missing.toml")).is_err());
}
