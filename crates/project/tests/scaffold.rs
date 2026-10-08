#![cfg(feature = "scaffold")]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const GAME: &str = "probe-game";
const TOOLS_MARKER: &str =
    "pfx-game tools build: the launcher's commands and pfx's editor are linked in";

fn workspace() -> PathBuf {
    std::path::absolute(Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")).unwrap()
}

fn packages() -> Vec<(String, PathBuf)> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(workspace().join("crates")).unwrap() {
        let folder = entry.unwrap().path();
        let Ok(text) = std::fs::read_to_string(folder.join("Cargo.toml")) else {
            continue;
        };
        let manifest: toml::Table = text.parse().unwrap();
        if let Some(name) = manifest
            .get("package")
            .and_then(|package| package.get("name"))
            .and_then(toml::Value::as_str)
        {
            found.push((name.to_string(), folder));
        }
    }
    found.sort();
    found
}

fn patched(manifest: &str) -> String {
    let mut text = manifest.to_string();
    if !text.contains("[workspace]") {
        text.push_str("\n[workspace]\n");
    }
    text.push_str(&format!(
        "\n[patch.\"{}\"]\n",
        pfx_project::scaffold::SOURCE
    ));
    for (name, folder) in packages() {
        text.push_str(&format!(
            "{name} = {{ path = {:?} }}\n",
            folder.display().to_string()
        ));
    }
    text
}

fn game(name: &str) -> String {
    format!("{GAME}-{name}")
}

fn scaffolded(name: &str) -> PathBuf {
    scaffolded_with(name, false)
}

fn scaffolded_with(name: &str, module: bool) -> PathBuf {
    let root = workspace().join("tmp/scaffold").join(name);
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    pfx_project::scaffold::scaffold_with(&game(name), &root, env!("CARGO_PKG_VERSION"), module)
        .unwrap();
    let manifest = root.join("Cargo.toml");
    let text = std::fs::read_to_string(&manifest).unwrap();
    std::fs::write(&manifest, patched(&text)).unwrap();
    root
}

fn cargo(root: &Path, args: &[&str]) -> Output {
    let output = Command::new(env!("CARGO"))
        .args(args)
        .current_dir(root)
        .env("CARGO_TARGET_DIR", workspace().join("tmp/scaffold-target"))
        .env("CARGO_NET_GIT_FETCH_WITH_CLI", "true")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "cargo {}:\n{}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn binary(name: &str) -> PathBuf {
    workspace().join("tmp/scaffold-target/debug").join(format!(
        "{}{}",
        game(name),
        std::env::consts::EXE_SUFFIX
    ))
}

fn carries(binary: &[u8], needle: &str) -> bool {
    binary
        .windows(needle.len())
        .any(|window| window == needle.as_bytes())
}

#[test]
fn the_test_patch_points_every_pfx_crate_at_this_workspace() {
    let text = patched("[package]\nname = \"x\"\n");
    assert!(text.contains("[workspace]\n"));
    assert!(text.contains(&format!("[patch.\"{}\"]", pfx_project::scaffold::SOURCE)));
    for name in [
        "pfx-game",
        "pfx-editor",
        "pfx-project",
        "pfx-gpu",
        "pfx-live",
        "pfx-load",
        "pfx-editor-shell",
        "pfx-editor-viewport",
    ] {
        assert_eq!(
            text.matches(&format!("\n{name} = {{ path = ")).count(),
            1,
            "{name}"
        );
    }
}

#[test]
#[ignore = "builds the scaffold and the whole engine twice; run by hand"]
fn the_scaffold_builds_with_and_without_tools_and_only_tools_links_the_editor() {
    let root = scaffolded("build");
    let game = game("build");
    cargo(&root, &["build", "-q", "--no-default-features"]);
    let built = std::fs::read(binary("build")).unwrap();
    for needle in ["pfx_editor", "egui::", "pfx_editor_shell", TOOLS_MARKER] {
        assert!(
            !carries(&built, needle),
            "the build without tools links {needle}"
        );
    }
    cargo(&root, &["build", "-q"]);
    let tools = std::fs::read(binary("build")).unwrap();
    if !cfg!(windows) {
        for needle in ["pfx_editor", "egui::", "pfx_editor_shell", TOOLS_MARKER] {
            assert!(carries(&tools, needle), "the tools build carries {needle}");
        }
    }
    let help = Command::new(binary("build")).arg("help").output().unwrap();
    assert!(help.status.success());
    let help = String::from_utf8(help.stdout).unwrap();
    assert!(help.starts_with(&format!("{game} 0.1.0\n")), "{help}");
    assert!(
        help.contains("  edit     opens the game in pfx's editor; F5 plays it\n"),
        "{help}"
    );
    let version = Command::new(binary("build"))
        .arg("version")
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8(version.stdout).unwrap(),
        format!("{game} 0.1.0\n")
    );
    let refused = Command::new(binary("build"))
        .args(["edit", "extra"])
        .output()
        .unwrap();
    assert_eq!(refused.status.code(), Some(2));
    assert_eq!(
        String::from_utf8_lossy(&refused.stderr),
        format!(
            "error: unexpected argument 'extra' found\n\nUsage: {game} edit\n\nFor more information, try '--help'.\n"
        )
    );
}

#[test]
#[ignore = "builds the scaffold's own tests and the engine under them; run by hand before the GPU run"]
fn the_scaffolded_games_tests_build() {
    let root = scaffolded("tests");
    cargo(&root, &["test", "-q", "--test", "first", "--no-run"]);
}

#[test]
#[ignore = "runs the scaffold's first scene on the GPU; run under pgpu after the build"]
fn the_scaffolded_game_runs_headless_and_shows_its_world() {
    let root = scaffolded("headless");
    let output = cargo(
        &root,
        &[
            "test",
            "-q",
            "--test",
            "first",
            "--",
            "--ignored",
            "--test-threads=1",
        ],
    );
    let out = String::from_utf8_lossy(&output.stdout);
    assert!(out.contains("1 passed"), "{out}");
    let shot = root.join("tmp/first-frame.png");
    let copy = workspace().join("tmp/scaffold/first-frame.png");
    std::fs::copy(&shot, &copy).unwrap();
}

#[test]
#[ignore = "builds the scaffold with its gameplay module and the engine; run by hand"]
fn the_scaffold_with_a_module_builds_and_its_module_compiles() {
    let root = scaffolded_with("module", true);
    cargo(&root, &["build", "-q"]);
    cargo(
        &root,
        &["check", "-q", "-p", &format!("{}-module", game("module"))],
    );
    let help = Command::new(binary("module")).arg("help").output().unwrap();
    assert!(help.status.success());
}

#[test]
#[ignore = "needs the wasm32-unknown-unknown target (rustup target add wasm32-unknown-unknown); run by hand"]
fn the_scaffolds_module_builds_to_wasm_and_the_host_swaps_it_keeping_its_state() {
    use pfx_mod::wasmtime::Store;
    use pfx_mod::wasmtime::component::{Instance, TypedFunc};
    use pfx_mod::{Api, Host, Limits, Mod, ModCtx, Swapped};

    type Tick = TypedFunc<(u64,), (u32,)>;
    fn bind(store: &mut Store<ModCtx<()>>, instance: &Instance) -> pfx_mod::wasmtime::Result<Tick> {
        instance.get_typed_func::<(u64,), (u32,)>(store, "tick")
    }

    let root = scaffolded_with("wasm", true);
    let module = format!("{}-module", game("wasm"));
    cargo(
        &root,
        &[
            "build",
            "-q",
            "-p",
            &module,
            "--target",
            "wasm32-unknown-unknown",
            "--release",
        ],
    );
    let built = workspace()
        .join("tmp/scaffold-target")
        .join(pfx_project::scaffold::module_path(&game("wasm")).trim_start_matches("target/"));
    let wasm = std::fs::read(&built).unwrap();
    let host = Host::<()>::new(Api::new("gameplay", "0.1.0"), Limits::default()).unwrap();
    let package = host.load_module("probe_game_module", &wasm).unwrap();
    let mut m = Mod::start(&host, package.clone(), bind, ());
    let tick =
        |m: &mut Mod<(), Tick>, n: u64| m.call(|f, store| f.call(store, (n,))).unwrap().value.0;
    assert_eq!((tick(&mut m, 0), tick(&mut m, 1)), (1, 2));
    assert_eq!(
        m.reload(&host, package).unwrap(),
        Swapped::Kept { bytes: 4 }
    );
    assert_eq!(tick(&mut m, 2), 3);
}
