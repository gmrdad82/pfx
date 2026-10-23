use std::process::Command;

const LIBRARIES: [&str; 6] = [
    "pfx-game",
    "pfx-live",
    "pfx-bake",
    "pfx-geom",
    "pfx-splash",
    "pfx-sound",
];

const NATIVE: [&str; 2] = ["meshopt", "cc"];

const C_BUILDS_THAT_STAY_OFF: [&str; 2] = ["android-activity", "wayland-backend"];

fn tree(args: &[&str]) -> Result<Vec<String>, String> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let output = Command::new(cargo)
        .args([
            "tree",
            "--offline",
            "--locked",
            "--target",
            "all",
            "--prefix",
            "none",
        ])
        .args(args)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .unwrap();
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned());
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(String::from)
        .collect())
}

fn names(lines: &[String]) -> Vec<&str> {
    lines
        .iter()
        .filter_map(|line| line.split_whitespace().next())
        .collect()
}

fn normal(package: &str, features: &[&str]) -> Vec<String> {
    let mut args = vec!["-e", "normal", "-p", package];
    args.extend_from_slice(features);
    tree(&args).unwrap_or_else(|error| panic!("cargo tree -p {package}: {error}"))
}

fn builds_c(package: &str, features: &[&str]) -> Vec<String> {
    let mut args = vec![
        "-e",
        "normal,build",
        "-p",
        package,
        "-i",
        "cc",
        "--depth",
        "1",
    ];
    args.extend_from_slice(features);
    match tree(&args) {
        Ok(lines) => names(&lines)
            .into_iter()
            .filter(|name| *name != "cc" && !name.starts_with('['))
            .map(String::from)
            .collect(),
        Err(error) if error.contains("did not match any packages") => Vec::new(),
        Err(error) => panic!("cargo tree -p {package} -i cc: {error}"),
    }
}

fn wayland_log(package: &str) -> bool {
    match tree(&["-e", "features", "-p", package, "-i", "wayland-backend"]) {
        Ok(lines) => lines
            .iter()
            .any(|line| line.starts_with("wayland-backend feature \"log\"")),
        Err(error) if error.contains("did not match any packages") => false,
        Err(error) => panic!("cargo tree -p {package} -i wayland-backend: {error}"),
    }
}

#[test]
fn no_library_crate_links_a_native_build_with_its_default_features() {
    let mut found = Vec::new();
    for package in LIBRARIES {
        let lines = normal(package, &[]);
        assert!(
            names(&lines).contains(&package),
            "{package} missing from its own tree"
        );
        for name in names(&lines) {
            if NATIVE.contains(&name) {
                found.push(format!("{package} pulls {name}"));
            }
        }
    }
    assert!(found.is_empty(), "{}", found.join("\n"));
}

#[test]
fn no_library_crate_compiles_c_or_cpp_with_its_default_features() {
    let mut found = Vec::new();
    for package in LIBRARIES {
        for builder in builds_c(package, &[]) {
            if !C_BUILDS_THAT_STAY_OFF.contains(&builder.as_str()) {
                found.push(format!("{package} builds C through {builder}"));
            }
        }
        if wayland_log(package) {
            found.push(format!(
                "{package} turns on wayland-backend's log shim, which compiles C"
            ));
        }
    }
    assert!(found.is_empty(), "{}", found.join("\n"));
}

#[test]
fn the_meshopt_feature_brings_the_cpp_simplifier_in() {
    let lines = normal("pfx-geom", &["--features", "meshopt"]);
    assert!(names(&lines).contains(&"meshopt"));
    assert!(builds_c("pfx-geom", &["--features", "meshopt"]).contains(&"meshopt".to_string()));
    let lines = normal("pfx-game", &["--features", "meshopt"]);
    assert!(names(&lines).contains(&"meshopt"));
}

#[test]
fn the_pfx_tool_keeps_the_simplifier_on() {
    let lines = normal("pfx", &[]);
    assert!(names(&lines).contains(&"meshopt"));
}
