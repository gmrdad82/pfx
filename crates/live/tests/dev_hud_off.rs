#![cfg(not(windows))]

use std::process::Command;

fn tree(features: &[&str]) -> Vec<String> {
    let mut command = Command::new(env!("CARGO"));
    command.current_dir(env!("CARGO_MANIFEST_DIR")).args([
        "tree",
        "-p",
        "pfx-live",
        "-e",
        "features",
        "--prefix",
        "none",
        "--offline",
    ]);
    command.args(features);
    let out = command.output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
}

fn egui(lines: &[String]) -> Vec<&String> {
    lines
        .iter()
        .filter(|line| {
            line.starts_with("egui")
                || line.starts_with("epaint")
                || line.starts_with("pfx-theme")
                || line.starts_with("pfx-editor-style")
                || line.starts_with("pfx-live feature \"dev-hud\"")
        })
        .collect()
}

#[test]
fn without_the_feature_nothing_of_egui_links_into_pfx_live() {
    let off = tree(&[]);
    assert!(off.iter().any(|line| line.starts_with("pfx-live ")));
    assert!(egui(&off).is_empty(), "{:?}", egui(&off));
    let on = tree(&["--features", "dev-hud"]);
    let linked = egui(&on);
    assert!(
        linked.iter().any(|line| line.starts_with("egui-wgpu ")),
        "{linked:?}"
    );
    assert!(
        linked.iter().any(|line| line.starts_with("pfx-theme ")),
        "{linked:?}"
    );
}
