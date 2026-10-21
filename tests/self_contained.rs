use std::fs;
use std::ops::Range;
use std::path::{Component, Path, PathBuf};

const LEAVES: [(&str, &str, &str); 1] = [(
    "crates/bake/build.rs",
    "../..",
    "folds the workspace's Cargo.lock and its path dependencies, or their vendored copies, into the bake build identity when they are there, and builds without them",
)];

#[derive(Debug, PartialEq)]
enum Embed {
    File(String),
    Generated,
    Unread(String),
}

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn members(root: &Path) -> Vec<String> {
    let manifest = fs::read_to_string(root.join("Cargo.toml")).expect("Cargo.toml");
    let line = manifest
        .lines()
        .find(|line| line.trim_start().starts_with("members"))
        .expect("Cargo.toml: its workspace members line");
    line.split('"')
        .skip(1)
        .step_by(2)
        .map(str::to_string)
        .collect()
}

fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            sources(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

fn normal(path: &Path) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::ParentDir => {
                if !out.pop() {
                    return None;
                }
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    Some(out)
}

fn literal(text: &str) -> Option<(&str, &str)> {
    let rest = text.trim_start().strip_prefix('"')?;
    let end = rest.find('"')?;
    Some((&rest[..end], &rest[end + 1..]))
}

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn argument(rest: &str) -> Embed {
    if let Some((path, _)) = literal(rest) {
        return Embed::File(path.to_string());
    }
    let shown = rest.lines().next().unwrap_or("").trim().to_string();
    let Some(inner) = rest.trim_start().strip_prefix("concat!(") else {
        return Embed::Unread(shown);
    };
    let inner = inner.trim_start();
    if inner.starts_with("env!(\"OUT_DIR\")") {
        return Embed::Generated;
    }
    let Some(mut tail) = inner.strip_prefix("env!(\"CARGO_MANIFEST_DIR\")") else {
        return Embed::Unread(shown);
    };
    let mut path = String::new();
    while let Some(next) = tail.trim_start().strip_prefix(',') {
        let Some((part, after)) = literal(next) else {
            break;
        };
        path.push_str(part);
        tail = after;
    }
    match path.strip_prefix('/') {
        Some(path) => Embed::File(format!("{{crate}}/{path}")),
        None => Embed::Unread(shown),
    }
}

fn embeds(text: &str) -> Vec<Embed> {
    let (literals, comments) = lex(text);
    let quiet = |at: usize| {
        literals
            .iter()
            .map(|literal| &literal.span)
            .chain(&comments)
            .any(|span| span.contains(&at))
    };
    let mut found = Vec::new();
    for (at, _) in text.match_indices("include") {
        if quiet(at) || text[..at].chars().next_back().is_some_and(is_ident) {
            continue;
        }
        let rest = &text[at + "include".len()..];
        let name = rest.chars().take_while(|c| is_ident(*c)).count();
        if name > 0 && !rest.starts_with('_') {
            continue;
        }
        let Some(rest) = rest[name..].strip_prefix("!(") else {
            continue;
        };
        found.push(argument(rest));
    }
    for (at, _) in text.match_indices("#[path = ") {
        if quiet(at) {
            continue;
        }
        if let Some((path, _)) = literal(&text[at + "#[path = ".len()..]) {
            found.push(Embed::File(path.to_string()));
        }
    }
    found
}

fn escapes(member: &str, file: &str, embed: &Embed) -> Option<String> {
    let path = match embed {
        Embed::Generated => return None,
        Embed::Unread(shown) => {
            return Some(format!(
                "{file}: embeds {shown}, a path this check can't read; give it a string literal or concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/…\")"
            ));
        }
        Embed::File(path) => path,
    };
    let joined = match path.strip_prefix("{crate}/") {
        Some(inside) => Path::new(member).join(inside),
        None => Path::new(file)
            .parent()
            .expect("a file's folder")
            .join(path),
    };
    match normal(&joined) {
        Some(resolved) if resolved.starts_with(member) => None,
        Some(resolved) => Some(format!(
            "{file}: embeds {path}, which is {} outside {member}/; copy it into the crate",
            resolved.display()
        )),
        None => Some(format!(
            "{file}: embeds {path}, which leaves the workspace; copy it into {member}/"
        )),
    }
}

struct Literal {
    span: Range<usize>,
    value: String,
}

fn quoted(text: &str, open: usize) -> (usize, String) {
    let mut value = String::new();
    let mut chars = text[open..].char_indices();
    while let Some((at, c)) = chars.next() {
        match c {
            '"' => return (open + at + 1, value),
            '\\' => {
                if let Some((_, next)) = chars.next() {
                    value.push('\\');
                    value.push(next);
                }
            }
            c => value.push(c),
        }
    }
    (text.len(), value)
}

fn lex(text: &str) -> (Vec<Literal>, Vec<Range<usize>>) {
    let bytes = text.as_bytes();
    let mut literals = Vec::new();
    let mut comments = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        match bytes[at] {
            b'/' if bytes.get(at + 1) == Some(&b'/') => {
                let end = text[at..].find('\n').map_or(bytes.len(), |n| at + n);
                comments.push(at..end);
                at = end;
            }
            b'\'' => {
                let rest = &text[at + 1..];
                at += match rest.chars().next() {
                    Some('\\') => rest
                        .get(2..)
                        .and_then(|tail| tail.find('\''))
                        .map_or(1, |n| n + 4),
                    Some(c) if rest[c.len_utf8()..].starts_with('\'') => c.len_utf8() + 2,
                    _ => 1,
                };
            }
            b'"' => {
                let hashes = text[..at].bytes().rev().take_while(|b| *b == b'#').count();
                let open = at + 1;
                let (end, value) = if text[..at - hashes].ends_with('r') {
                    let closing = format!("\"{}", "#".repeat(hashes));
                    let n = text[open..].find(&closing).unwrap_or(text.len() - open);
                    (
                        (open + n + closing.len()).min(text.len()),
                        text[open..open + n].to_string(),
                    )
                } else {
                    quoted(text, open)
                };
                literals.push(Literal {
                    span: at..end,
                    value,
                });
                at = end;
            }
            _ => at += 1,
        }
    }
    (literals, comments)
}

fn build_script_leaves(text: &str) -> Vec<String> {
    lex(text)
        .0
        .into_iter()
        .map(|literal| literal.value)
        .filter(|value| value.contains("../") || value == ".." || value.ends_with("/.."))
        .collect()
}

fn check(root: &Path) -> Vec<String> {
    let mut problems = Vec::new();
    for member in members(root) {
        let mut files = Vec::new();
        sources(&root.join(&member).join("src"), &mut files);
        let script = root.join(&member).join("build.rs");
        if script.is_file() {
            files.push(script.clone());
        }
        files.sort();
        for path in files {
            let file = path
                .strip_prefix(root)
                .expect("inside the workspace")
                .to_string_lossy()
                .into_owned();
            let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{file}: {e}"));
            for embed in embeds(&text) {
                if let Some(problem) = escapes(&member, &file, &embed) {
                    problems.push(problem);
                } else if let Embed::File(target) = &embed {
                    let joined = match target.strip_prefix("{crate}/") {
                        Some(inside) => root.join(&member).join(inside),
                        None => path.parent().expect("a file's folder").join(target),
                    };
                    if !joined.is_file() {
                        problems.push(format!("{file}: embeds {target}, which is not there"));
                    }
                }
            }
            if path == script {
                for value in build_script_leaves(&text) {
                    let allowed = LEAVES
                        .iter()
                        .any(|(at, leaves, _)| *at == file && *leaves == value);
                    if !allowed {
                        problems.push(format!(
                            "{file}: reads {value:?}, outside {member}/; a crate's build script reads only its own folder"
                        ));
                    }
                }
            }
        }
    }
    for (file, value, why) in LEAVES {
        let text = fs::read_to_string(root.join(file)).unwrap_or_default();
        if !build_script_leaves(&text).iter().any(|v| v == value) {
            problems.push(format!(
                "{file} no longer reads {value:?} ({why}); drop it from LEAVES"
            ));
        }
    }
    problems
}

fn copies(root: &Path) -> Vec<String> {
    let mut problems = Vec::new();
    for member in members(root) {
        let assets = root.join(&member).join("assets");
        let Ok(source) = fs::read_to_string(assets.join("SOURCE")) else {
            continue;
        };
        let mut named = Vec::new();
        for line in source.lines().filter(|line| !line.trim().is_empty()) {
            let Some((name, rest)) = line.split_once(" is ") else {
                problems.push(format!(
                    "{member}/assets/SOURCE: {line:?} is not `<file> is <path>, …`"
                ));
                continue;
            };
            let origin = rest.split(',').next().unwrap_or(rest).trim();
            named.push(name.to_string());
            let copy = fs::read(assets.join(name));
            let master = fs::read(root.join(origin));
            match (copy, master) {
                (Ok(copy), Ok(master)) if copy == master => {}
                (Ok(_), Ok(_)) => problems.push(format!(
                    "{member}/assets/{name} differs from {origin}; rerun bin/marks"
                )),
                (Err(e), _) => problems.push(format!("{member}/assets/{name}: {e}")),
                (_, Err(e)) => problems.push(format!("{origin}: {e}")),
            }
        }
        for entry in fs::read_dir(&assets)
            .expect("the crate's assets folder")
            .flatten()
        {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name != "SOURCE" && !named.contains(&name) {
                problems.push(format!(
                    "{member}/assets/{name}: not named in {member}/assets/SOURCE"
                ));
            }
        }
    }
    problems
}

#[test]
fn every_crate_embeds_only_files_inside_its_own_folder() {
    let problems = check(&root());
    assert!(
        problems.is_empty(),
        "a vendored build copies each crate's folder alone, so a crate embeds nothing outside it:\n{}",
        problems.join("\n")
    );
}

#[test]
fn every_crate_copy_of_a_workspace_file_matches_it() {
    let problems = copies(&root());
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn the_check_reads_each_way_a_crate_embeds_a_file() {
    let text = r###"
static A: &str = include_str!("../../../assets/marks/a.svg");
static B: &[u8] = include_bytes!(
    "../assets/b.png"
);
static C: &str = include_str!(concat!(env!("OUT_DIR"), "/c.rs"));
static D: &str = std::include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../d.toml"));
static E: &str = include_str!(NAME);
static F: &str = wgpu::include_wgsl!("../../other/f.wgsl");
#[path = "../../other/g.rs"]
mod g;
fn not_an_embed() { included!("x"); include_me("y"); }
const T: &str = "x.font(include_bytes!(\"../../no.ttf\"))";
const U: &str = r##"include_str!("../../no.txt") "# #[path = "no.rs"]"##;
"###;
    let found = embeds(text);
    assert_eq!(
        found,
        [
            Embed::File("../../../assets/marks/a.svg".into()),
            Embed::File("../assets/b.png".into()),
            Embed::Generated,
            Embed::File("{crate}/../../d.toml".into()),
            Embed::Unread("NAME);".into()),
            Embed::File("../../other/f.wgsl".into()),
            Embed::File("../../other/g.rs".into()),
        ]
    );
    let member = "crates/splash";
    let file = "crates/splash/src/art.rs";
    let left: Vec<bool> = found
        .iter()
        .map(|embed| escapes(member, file, embed).is_some())
        .collect();
    assert_eq!(left, [true, false, false, true, true, true, true]);
    assert_eq!(
        build_script_leaves(
            r#"let q = ['"', '\'']; println!("cargo:rerun-if-changed=../../assets/x.ico"); let s = "assets/x.ico"; here.join("../..");"#
        ),
        ["cargo:rerun-if-changed=../../assets/x.ico", "../.."]
    );
}
