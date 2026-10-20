#![cfg(target_os = "linux")]

use std::os::unix::process::ExitStatusExt;
use std::path::PathBuf;
use std::process::Command;

const RAW: u64 = 3 << 30;
const LIMIT: u64 = 1 << 30;

fn oversized() -> Vec<u8> {
    let stored = RAW / 1032 + 1;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"PFXFILE\0");
    bytes.extend_from_slice(&1u32.to_le_bytes());
    bytes.extend_from_slice(&1u32.to_le_bytes());
    bytes.extend_from_slice(b"RCPE");
    bytes.extend_from_slice(&1u32.to_le_bytes());
    bytes.push(1);
    bytes.extend_from_slice(&64u64.to_le_bytes());
    bytes.extend_from_slice(&stored.to_le_bytes());
    bytes.extend_from_slice(&RAW.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.resize(64 + stored as usize, 0);
    bytes
}

#[test]
fn a_file_claiming_more_memory_than_the_limit_is_refused_with_exit_1_not_a_signal() {
    if Command::new("prlimit").arg("--version").output().is_err() {
        eprintln!("prlimit is not installed; skipped");
        return;
    }
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let base = root
        .join("tmp")
        .join(format!("memory-{}", std::process::id()));
    std::fs::create_dir_all(&base).unwrap();
    let file = base.join("oversized.pitofx");
    std::fs::write(&file, oversized()).unwrap();
    let output = Command::new("prlimit")
        .env("PFX_CACHE_DIR", "off")
        .arg("--core=1:1")
        .arg(format!("--as={LIMIT}:{LIMIT}"))
        .arg("--")
        .arg(env!("CARGO_BIN_EXE_pfx"))
        .args(["scene", "print"])
        .arg(&file)
        .output()
        .unwrap();
    std::fs::remove_dir_all(&base).ok();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.signal(), None, "{stderr}");
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert!(
        stderr.starts_with("pfx ") && stderr.contains(&format!("chunk RCPE needs {RAW} bytes: ")),
        "{stderr}"
    );
    assert_eq!(stderr.trim_end().lines().count(), 1, "{stderr}");
}
