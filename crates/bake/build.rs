use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

const SKIPPED: &[&str] = &["tests", "examples", "benches", "target", "tmp"];

fn listed(section: &str) -> bool {
    let kind = |name: &str| name == "dependencies" || name == "build-dependencies";
    kind(section)
        || (section.starts_with("target.")
            && section.rsplit_once('.').is_some_and(|(_, last)| kind(last)))
}

fn table(section: &str) -> Option<&str> {
    section
        .rsplit_once('.')
        .filter(|(head, _)| listed(head))
        .map(|(_, name)| name.trim_matches(['"', '\'']))
}

fn value(text: &str, key: &str) -> Option<String> {
    let mut rest = text;
    while let Some(at) = rest.find(key) {
        let before = rest[..at].chars().next_back();
        let after = rest[at + key.len()..].trim_start();
        rest = &rest[at + key.len()..];
        if before.is_some_and(|c| !matches!(c, '{' | ',' | ' ' | '\t')) {
            continue;
        }
        let Some(after) = after.strip_prefix('=') else {
            continue;
        };
        let after = after.trim_start();
        let quote = after.chars().next()?;
        if quote != '"' && quote != '\'' {
            return Some(after.split([',', '}']).next()?.trim().to_string());
        }
        return after[1..].split(quote).next().map(str::to_string);
    }
    None
}

fn dependency(dir: &Path, manifest: &Path, name: &str, spec: &str, out: &mut Vec<PathBuf>) {
    if let Some(path) = value(spec, "path") {
        let vendored = dir.parent().map(|parent| parent.join(name));
        out.extend(
            std::iter::once(dir.join(path))
                .chain(vendored)
                .find(|candidate| candidate.join("Cargo.toml").is_file()),
        );
        return;
    }
    let pinned = spec.trim_start().starts_with(['"', '\''])
        || value(spec, "git").is_some()
        || value(spec, "version").is_some();
    if name.starts_with("pfx-") && !pinned {
        panic!(
            "{}: the bake build identity cannot follow {name} = {}; give it a path, a git source or a version",
            manifest.display(),
            spec.trim()
        );
    }
}

fn path_deps(dir: &Path) -> Vec<PathBuf> {
    let manifest = dir.join("Cargo.toml");
    let text = fs::read_to_string(&manifest)
        .unwrap_or_else(|error| panic!("{}: {error}", manifest.display()));
    let mut section = String::new();
    let mut tabled = String::new();
    let mut out = Vec::new();
    let close = |section: &str, tabled: &mut String, out: &mut Vec<PathBuf>| {
        if let Some(name) = table(section) {
            dependency(dir, &manifest, name, &format!("{{ {tabled} }}"), out);
        }
        tabled.clear();
    };
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            close(&section, &mut tabled, &mut out);
            section = line.trim_matches(['[', ']']).trim().to_string();
            continue;
        }
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if table(&section).is_some() {
            tabled.push_str(line);
            tabled.push_str(", ");
        } else if listed(&section)
            && let Some((name, spec)) = line.split_once('=')
        {
            let name = name.trim().trim_matches(['"', '\'']);
            match name.split_once('.') {
                Some((name, field)) => dependency(
                    dir,
                    &manifest,
                    name,
                    &format!("{{ {field} = {spec} }}"),
                    &mut out,
                ),
                None => dependency(dir, &manifest, name, spec, &mut out),
            }
        }
    }
    close(&section, &mut tabled, &mut out);
    out
}

fn kept(name: &str) -> bool {
    !(name.starts_with('.')
        || SKIPPED.contains(&name)
        || name == "tests.rs"
        || name.ends_with("_tests.rs")
        || name.ends_with(".md"))
}

fn files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_symlink() || !kept(&entry.file_name().to_string_lossy()) {
            continue;
        }
        if kind.is_dir() {
            files(&entry.path(), out);
        } else if kind.is_file() {
            out.push(entry.path());
        }
    }
}

fn label(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .components()
        .map(|part| part.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

fn part(hash: &mut Sha256, bytes: &[u8]) {
    hash.update((bytes.len() as u64).to_le_bytes());
    hash.update(bytes);
}

fn sourced(lock: &str) -> Vec<&str> {
    lock.split("[[package]]")
        .skip(1)
        .map(str::trim)
        .filter(|block| block.lines().any(|line| line.starts_with("source = ")))
        .collect()
}

fn main() {
    let here = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets it"));
    let root = here
        .join("../..")
        .canonicalize()
        .unwrap_or(here.join("../.."));
    let mut seen = BTreeSet::new();
    let mut crates = BTreeMap::new();
    let mut todo = vec![here];
    while let Some(dir) = todo.pop() {
        let dir = dir.canonicalize().unwrap_or(dir);
        if seen.insert(dir.clone()) {
            todo.extend(path_deps(&dir));
            crates.insert(label(&dir, &root), dir);
        }
    }
    let mut hash = Sha256::new();
    let lock = root.join("Cargo.lock");
    if let Ok(text) = fs::read_to_string(&lock) {
        part(&mut hash, b"Cargo.lock");
        for block in sourced(&text) {
            part(&mut hash, block.as_bytes());
        }
        println!("cargo:rerun-if-changed={}", lock.display());
    }
    for dir in crates.values() {
        let mut found = Vec::new();
        files(dir, &mut found);
        let mut found: Vec<(String, PathBuf)> = found
            .into_iter()
            .map(|path| (label(&path, &root), path))
            .collect();
        found.sort();
        for (name, path) in found {
            println!("cargo:rerun-if-changed={}", path.display());
            if let Ok(bytes) = fs::read(&path) {
                part(&mut hash, name.as_bytes());
                part(&mut hash, &bytes);
            }
        }
    }
    println!(
        "cargo:rustc-env=PFX_BAKE_BUILD={}",
        hex::encode(hash.finalize())
    );
}
