use super::*;

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tmp/atomic-tests")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn a_partial_keeps_the_extension_and_hides_beside_the_file() {
    assert_eq!(
        partial(Path::new("d/frame-0003.png")),
        Path::new("d/.frame-0003.partial.png")
    );
    assert_eq!(partial(Path::new("audio")), Path::new(".audio.partial"));
    assert_eq!(
        partial(Path::new("d/.frame-0003.checkpoint")),
        Path::new("d/..frame-0003.partial.checkpoint")
    );
    for name in [
        ".frame-0003.partial.png",
        ".audio.partial",
        "..frame-0003.partial.checkpoint",
    ] {
        assert!(is_partial(name), "{name}");
    }
    for name in [
        "frame-0003.png",
        "partial.png",
        ".frame-0003.checkpoint",
        "clip.txt",
    ] {
        assert!(!is_partial(name), "{name}");
    }
}

#[test]
fn a_failed_write_leaves_neither_the_file_nor_its_partial() {
    let dir = scratch("failed");
    let path = dir.join("frame.png");
    std::fs::write(&path, b"before").unwrap();
    let failed: Result<(), String> = write_with(&path, |temp| {
        std::fs::write(temp, b"half").unwrap();
        Err("stopped".into())
    });
    assert_eq!(failed, Err("stopped".into()));
    assert_eq!(std::fs::read(&path).unwrap(), b"before");
    assert!(!partial(&path).exists());
    write(&path, b"after").unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"after");
    assert!(!partial(&path).exists());
    write(&dir.join("deeper/still.png"), b"x").unwrap();
    assert!(dir.join("deeper/still.png").is_file());
}

#[test]
fn a_sweep_removes_leftover_partials_and_keeps_everything_else() {
    let dir = scratch("sweep");
    for name in [
        ".frame-0001.partial.png",
        "..frame-0002.partial.checkpoint",
        ".audio.partial",
        "frame-0000.png",
        ".frame-0002.checkpoint",
        "clip.txt",
    ] {
        std::fs::write(dir.join(name), b"x").unwrap();
    }
    std::fs::create_dir(dir.join(".kept.partial")).unwrap();
    assert_eq!(
        sweep_matching(&dir, |name| name.contains("0001")).unwrap(),
        [dir.join(".frame-0001.partial.png")]
    );
    std::fs::write(dir.join(".frame-0001.partial.png"), b"x").unwrap();
    let gone = sweep(&dir).unwrap();
    assert_eq!(
        gone,
        [
            dir.join("..frame-0002.partial.checkpoint"),
            dir.join(".audio.partial"),
            dir.join(".frame-0001.partial.png"),
        ]
    );
    let mut left: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    left.sort();
    assert_eq!(
        left,
        [
            ".frame-0002.checkpoint",
            ".kept.partial",
            "clip.txt",
            "frame-0000.png"
        ]
    );
    assert!(sweep(&dir.join("missing")).unwrap().is_empty());
}
