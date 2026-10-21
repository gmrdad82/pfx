use std::path::{Component, Path, PathBuf};
use std::process::Command;

const RULE: &str = "In every Markdown file the repo tracks (*.md and *.md.in, CHANGELOG.md and TRADEMARKS.md excepted), a line that says \"powered by pfx\" (any case, once the characters [ ] * _ ` are dropped and runs of whitespace are one space) is a credit of pfx, and so is any line with an image (<picture, <img or ![) of a powered-by file. A credit line is exactly one <picture> element, alone on its line: <picture><source media=\"(prefers-color-scheme: dark)\" srcset=\"<dir>powered-by-<64|128>.png\"><img src=\"<dir>powered-by-light-<64|128>.png\" ... alt=\"Powered by pfx\"></picture>. No other line says \"powered by pfx\".";

const SIZES: [&str; 2] = ["64", "128"];

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn files(root: &Path) -> Vec<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args([
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ])
        .output()
        .expect("git ls-files");
    assert!(out.status.success(), "git ls-files failed");
    let mut names: Vec<String> = out
        .stdout
        .split(|b| *b == 0)
        .filter(|p| !p.is_empty())
        .map(|p| String::from_utf8_lossy(p).into_owned())
        .filter(|p| root.join(p).is_file())
        .collect();
    names.sort();
    names.dedup();
    names
}

fn pinned(root: &Path) -> Vec<String> {
    let script = read(&root.join("bin/marks"));
    let list = script
        .split("for file in ")
        .nth(1)
        .and_then(|rest| rest.split("; do").next())
        .expect("bin/marks: its `for file in …; do` list");
    list.split_whitespace()
        .filter(|word| *word != "\\")
        .map(str::to_string)
        .collect()
}

fn normalized(line: &str) -> String {
    line.chars()
        .filter(|c| !matches!(c, '[' | ']' | '*' | '_' | '`'))
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn shows_the_lockup(line: &str) -> bool {
    line.contains("powered-by")
        && ["<picture", "<img", "!["]
            .iter()
            .any(|opener| line.contains(opener))
}

fn is_credit(line: &str) -> bool {
    normalized(line).contains("powered by pfx") || shows_the_lockup(line)
}

fn attribute<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let at = tag.find(&format!(" {name}=\""))? + name.len() + 3;
    tag[at..].split('"').next()
}

fn named(path: &str, stem: &str) -> bool {
    let file = path.rsplit('/').next().unwrap_or(path);
    SIZES
        .iter()
        .any(|size| file == format!("{stem}-{size}.png"))
}

fn credit_problem(line: &str) -> Option<&'static str> {
    let line = line.trim();
    let Some(inner) = line
        .strip_prefix("<picture>")
        .and_then(|rest| rest.strip_suffix("</picture>"))
    else {
        return Some("not one <picture> element alone on its line");
    };
    if inner.contains("<picture>") {
        return Some("more than one <picture> on the line");
    }
    let Some(source) = inner
        .strip_prefix("<source ")
        .and_then(|rest| rest.split('>').next())
    else {
        return Some("the picture does not open with its <source>");
    };
    let source = format!(" {source}");
    if attribute(&source, "media") != Some("(prefers-color-scheme: dark)") {
        return Some("its <source> is not media=\"(prefers-color-scheme: dark)\"");
    }
    if !attribute(&source, "srcset").is_some_and(|src| named(src, "powered-by")) {
        return Some("its <source> srcset is not powered-by-64.png or powered-by-128.png");
    }
    let Some(img) = inner
        .find("<img ")
        .map(|at| &inner[at + 4..])
        .and_then(|rest| rest.split('>').next())
    else {
        return Some("the picture has no <img>");
    };
    if !attribute(img, "src").is_some_and(|src| named(src, "powered-by-light")) {
        return Some("its <img> src is not powered-by-light-64.png or powered-by-light-128.png");
    }
    if attribute(img, "alt") != Some("Powered by pfx") {
        return Some("its <img> alt is not \"Powered by pfx\"");
    }
    None
}

fn markdown(path: &str) -> bool {
    (path.ends_with(".md") || path.ends_with(".md.in"))
        && path != "CHANGELOG.md"
        && path != "TRADEMARKS.md"
}

fn credit_lines(text: &str) -> Vec<(usize, &str)> {
    text.lines()
        .enumerate()
        .filter(|(_, line)| is_credit(line))
        .map(|(number, line)| (number + 1, line))
        .collect()
}

fn resolve(base: &Path, relative: &str) -> PathBuf {
    let mut out = PathBuf::new();
    for part in base.join(relative).components() {
        match part {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

fn marks_copy(marks: &Path, resolved: &Path) -> Option<String> {
    if let Ok(name) = resolved.strip_prefix(marks) {
        return Some(name.to_string_lossy().into_owned());
    }
    let name = resolved.file_name()?.to_string_lossy().into_owned();
    let folder = resolved.parent()?;
    let crates = marks.parent()?.parent()?.join("crates");
    let in_crate = folder.file_name()? == "assets" && folder.parent()?.parent()? == crates;
    let same = std::fs::read(resolved).ok()? == std::fs::read(marks.join(&name)).ok()?;
    (in_crate && same).then_some(name)
}

fn lockup_includes(root: &Path, file: &str) -> Vec<String> {
    let path = root.join(file);
    let text = read(&path);
    let base = path.parent().expect("a file's folder");
    let marks = root.join("assets/marks");
    let mut found = Vec::new();
    for opener in ["include_str!(\"", "include_bytes!(\""] {
        for (at, _) in text.match_indices(opener) {
            let target = text[at + opener.len()..].split('"').next().unwrap_or("");
            if !target.contains("powered-by") {
                continue;
            }
            let resolved = resolve(base, target);
            let name = marks_copy(&marks, &resolved).unwrap_or_else(|| {
                panic!("{file}: {target} is not bin/marks' copy in assets/marks/ or in a crate's assets/")
            });
            found.push(name);
        }
    }
    found
}

#[test]
fn the_splash_draws_the_lockup_bin_marks_pins() {
    let root = root();
    let pinned = pinned(&root);
    for name in ["powered-by.svg", "powered-by-light.svg"] {
        assert!(pinned.iter().any(|p| p == name), "bin/marks pins no {name}");
    }
    let mut drawn = lockup_includes(&root, "crates/splash/src/art.rs");
    drawn.sort();
    assert_eq!(
        drawn,
        ["powered-by-light.svg", "powered-by.svg"],
        "the splash draws the Powered by lockup from bin/marks' copies, both variants"
    );
    let art = root.join("crates/splash/art");
    for entry in std::fs::read_dir(&art).expect("crates/splash/art") {
        let name = entry.expect("an entry").file_name();
        let name = name.to_string_lossy().to_lowercase();
        assert!(
            !name.contains("powered"),
            "crates/splash/art/{name}: the splash's credit is bin/marks' lockup alone"
        );
    }
    let source = read(&root.join("crates/splash/src/art.rs"));
    assert!(
        !source.contains("include_str!(\"../art/powered"),
        "the splash draws no Powered by artwork of its own"
    );
}

#[test]
fn every_markdown_credit_of_pfx_is_the_lockup_picture() {
    let root = root();
    let mut problems = Vec::new();
    let mut readme = 0;
    for path in files(&root).into_iter().filter(|p| markdown(p)) {
        let text = read(&root.join(&path));
        for (number, line) in credit_lines(&text) {
            match credit_problem(line) {
                Some(why) => problems.push(format!("{path}:{number}: {why}")),
                None if path == "README.md" => readme += 1,
                None => {}
            }
        }
    }
    assert!(
        problems.is_empty(),
        "{}\n\nThe rule: {RULE}",
        problems.join("\n")
    );
    assert!(
        readme > 0,
        "README.md carries no Powered by credit\n\nThe rule: {RULE}"
    );
}

#[test]
fn a_game_gets_the_same_lockup_files() {
    let root = root();
    let pinned = pinned(&root);
    for file in ["src/steam.rs", "crates/project/src/scaffold.rs"] {
        let mut names = lockup_includes(&root, file);
        names.sort();
        assert!(
            names.iter().any(|n| named(n, "powered-by"))
                && names.iter().any(|n| named(n, "powered-by-light")),
            "{file} hands a game no dark and light pair of the Powered by PNGs: {names:?}"
        );
        for name in &names {
            assert!(
                pinned.contains(name),
                "{file}: {name} is not pinned by bin/marks"
            );
        }
    }
    let scaffold = read(&root.join("crates/project/src/scaffold.rs"));
    let readme = read(&root.join("crates/project/src/templates/README.md.in"));
    let credits = credit_lines(&readme);
    assert_eq!(credits.len(), 1, "pfx new's README carries one credit");
    let (_, line) = credits[0];
    assert_eq!(credit_problem(line), None, "The rule: {RULE}");
    for tag in ["srcset", "src"] {
        let target = line
            .find(&format!(" {tag}=\""))
            .map(|at| &line[at + tag.len() + 3..])
            .and_then(|rest| rest.split('"').next())
            .expect("the credit's files");
        assert!(
            scaffold.contains(&format!("\"{target}\"")),
            "pfx new's README shows {target}, which the scaffold never writes"
        );
    }
}

#[test]
fn the_rule_takes_the_picture_and_refuses_anything_else() {
    let good = r#"<picture><source media="(prefers-color-scheme: dark)" srcset="assets/marks/powered-by-128.png"><img src="assets/marks/powered-by-light-128.png" height="64" alt="Powered by pfx"></picture>"#;
    assert!(is_credit(good));
    assert_eq!(credit_problem(good), None);
    for plain in [
        "Powered by pfx, a game engine.",
        "Powered by [pfx](https://pitomd.com/pfx).",
        "**Powered  by** `pfx`",
        "![pfx](assets/marks/powered-by-128.png)",
    ] {
        assert!(is_credit(plain), "{plain}");
        assert!(credit_problem(plain).is_some(), "{plain}");
    }
    assert!(credit_problem(&good.replace("powered-by-light-128", "powered-by-128")).is_some());
    assert!(
        credit_problem(&good.replace("(prefers-color-scheme: dark)", "(min-width: 1px)")).is_some()
    );
    assert!(!is_credit(
        "the Powered by lockup, `powered-by-light.svg` on a light ground"
    ));
}
