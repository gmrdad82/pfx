use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

fn base(name: &str) -> PathBuf {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tmp")
        .join(format!("compose-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("samples/chase.pitofx"),
        base.join("chase.pitofx"),
    )
    .unwrap();
    base
}

fn compose(base: &Path, cache: &Path) -> Output {
    let home = base.join("home");
    Command::new(env!("CARGO_BIN_EXE_pfx"))
        .env("PFX_CACHE_DIR", cache)
        .env("HOME", &home)
        .env("XDG_CACHE_HOME", home.join("xdg"))
        .env("LOCALAPPDATA", home.join("local"))
        .args([
            "compose",
            base.join("chase.pitofx").to_str().unwrap(),
            "--style",
            "build_drop",
            "--seed",
            "3",
            "--seconds",
            "4",
            "--wav",
            base.join("out.wav").to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap()
}

fn line(output: &Output) -> Value {
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(files(&path));
        } else {
            out.push(path);
        }
    }
    out.sort();
    out
}

#[test]
fn compose_with_the_cache_off_reads_and_writes_no_cache() {
    let base = base("off");
    let first = line(&compose(&base, Path::new("off")));
    assert_eq!(first["cached"], false);
    let wav = std::fs::read(base.join("out.wav")).unwrap();
    assert_eq!(&wav[..4], b"RIFF");
    let second = line(&compose(&base, Path::new("off")));
    assert_eq!(second["cached"], false);
    assert!(std::fs::read(base.join("out.wav")).unwrap() == wav);
    assert!(files(&base.join("home")).is_empty());
    let _ = std::fs::remove_dir_all(&base);
}

#[cfg(unix)]
#[test]
fn compose_refuses_a_master_in_a_cache_others_can_write() {
    use std::os::unix::fs::PermissionsExt;
    let base = base("shared");
    let cache = base.join("cache");
    let first = line(&compose(&base, &cache));
    assert_eq!(first["cached"], false);
    assert_eq!(line(&compose(&base, &cache))["cached"], true);
    let kept = files(&cache);
    assert!(
        kept.iter()
            .any(|path| path.starts_with(cache.join("compose")))
    );
    let before: Vec<Vec<u8>> = kept.iter().map(|p| std::fs::read(p).unwrap()).collect();
    std::fs::set_permissions(&cache, std::fs::Permissions::from_mode(0o770)).unwrap();
    let output = compose(&base, &cache);
    let refused = line(&output);
    assert_eq!(refused["cached"], false);
    assert!(String::from_utf8_lossy(&output.stderr).contains("other users can write in it"));
    assert!(files(&cache) == kept);
    let after: Vec<Vec<u8>> = kept.iter().map(|p| std::fs::read(p).unwrap()).collect();
    assert!(after == before);
    assert!(files(&base.join("home")).is_empty());
    let _ = std::fs::remove_dir_all(&base);
}
