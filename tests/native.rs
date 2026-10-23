use std::process::Command;

const LIBRARIES: [&str; 10] = [
    "pfx-game",
    "pfx-live",
    "pfx-play",
    "pfx-runtime",
    "pfx-trace",
    "pfx-assets",
    "pfx-bake",
    "pfx-geom",
    "pfx-splash",
    "pfx-sound",
];

const NATIVE: [&str; 2] = ["meshopt", "cc"];

const C_BUILDS_THAT_STAY_OFF: [&str; 3] = [
    "android-activity",
    "iana-time-zone-haiku",
    "wayland-backend",
];

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

fn specs(error: &str, spec: &str) -> Vec<String> {
    let tail = spec.rsplit('#').next().unwrap_or(spec);
    let name = tail.split('@').next().unwrap_or(tail);
    let bare = format!("{name}@");
    let qualified = format!("#{name}@");
    error
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with(&bare) || line.contains(&qualified))
        .filter(|line| *line != spec)
        .map(String::from)
        .collect()
}

fn inverted(package: &str, edges: &str, spec: &str, extra: &[&str]) -> Vec<String> {
    let mut args = vec!["-e", edges, "-p", package, "-i", spec];
    args.extend_from_slice(extra);
    match tree(&args) {
        Ok(lines) => lines,
        Err(error) if error.contains("did not match any packages") => Vec::new(),
        Err(error) if error.contains("is ambiguous") => {
            let versions = specs(&error, spec);
            assert!(!versions.is_empty(), "cargo tree -p {package} -i {spec}: {error}");
            versions
                .iter()
                .flat_map(|version| inverted(package, edges, version, extra))
                .collect()
        }
        Err(error) => panic!("cargo tree -p {package} -i {spec}: {error}"),
    }
}

fn builds_c(package: &str, features: &[&str]) -> Vec<String> {
    let mut extra = vec!["--depth", "1"];
    extra.extend_from_slice(features);
    let lines = inverted(package, "normal,build", "cc", &extra);
    let mut builders: Vec<String> = names(&lines)
        .into_iter()
        .filter(|name| *name != "cc" && !name.starts_with('['))
        .map(String::from)
        .collect();
    builders.sort();
    builders.dedup();
    builders
}

fn wayland_log(package: &str) -> bool {
    inverted(package, "features", "wayland-backend", &[])
        .iter()
        .any(|line| line.starts_with("wayland-backend feature \"log\""))
}

#[test]
fn no_library_crate_pulls_meshopt_or_cc_with_its_default_features() {
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

fn ambiguous(spec: &str, choices: &[&str]) -> String {
    let mut lines = vec![
        format!("error: specification `{spec}` is ambiguous"),
        "help: re-run this command with one of the following specifications".to_string(),
    ];
    lines.extend(choices.iter().map(|choice| format!("  {choice}")));
    lines.join("\n")
}

#[test]
fn an_ambiguous_package_splits_into_each_version_cargo_names() {
    let error = ambiguous("cc", &["cc@1.2.3", "cc@1.5.1"]);
    assert_eq!(specs(&error, "cc"), ["cc@1.2.3", "cc@1.5.1"]);
}

#[test]
fn a_version_from_two_sources_splits_into_qualified_specifications() {
    let registry = "registry+https://github.com/rust-lang/crates.io-index#cc@1.5.1";
    let git = "git+https://example.com/cc#cc@1.5.1";
    let error = ambiguous("cc@1.5.1", &[registry, git]);
    assert_eq!(specs(&error, "cc@1.5.1"), [registry, git]);
    assert_eq!(specs(&error, registry), [git]);
}

#[test]
fn an_ambiguity_without_choices_or_for_another_package_gives_none() {
    assert!(specs(&ambiguous("cc", &[]), "cc").is_empty());
    let other = ambiguous("ccx", &["ccx@0.1.0", "ccx@0.2.0", "base64@0.13.1"]);
    assert!(specs(&other, "cc").is_empty());
    assert_eq!(specs(&other, "ccx"), ["ccx@0.1.0", "ccx@0.2.0"]);
}
