use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const OLD: &str = concat!("p", "gpu");

const PRIVATE_FILES: [&str; 7] = [
    "AGENTS.md",
    "CONVENTIONS.md",
    "HANDOVER.md",
    "DESIGN.md",
    "ROADMAP.md",
    "pito-app.toml",
    "prompts/common.md",
];

const RUST_ROOTS: [&str; 4] = ["src/", "crates/", "tests/", "examples/"];

#[derive(Debug, PartialEq, Eq)]
struct Finding {
    path: String,
    line: usize,
    text: String,
}

fn is_word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

fn allowed(text: &[char], start: usize, end: usize) -> bool {
    let before: String = text[..start].iter().collect();
    let after: String = text[end..].iter().collect();
    before.ends_with("mod ") || after.starts_with("::") || after.starts_with(".rs")
}

fn word_hits(text: &str) -> Vec<usize> {
    let chars: Vec<char> = text.chars().collect();
    let word: Vec<char> = OLD.chars().collect();
    let mut hits = Vec::new();
    let mut at = 0;
    while at + word.len() <= chars.len() {
        let end = at + word.len();
        if chars[at..end] == word[..]
            && (at == 0 || !is_word(chars[at - 1]))
            && (end == chars.len() || !is_word(chars[end]))
            && !allowed(&chars, at, end)
        {
            hits.push(at);
        }
        at += 1;
    }
    hits
}

fn scan_text(path: &str, text: &str) -> Vec<Finding> {
    text.lines()
        .enumerate()
        .filter(|(_, line)| !word_hits(line).is_empty())
        .map(|(at, line)| Finding {
            path: path.to_string(),
            line: at + 1,
            text: line.trim().to_string(),
        })
        .collect()
}

struct Literal {
    line: usize,
    body: String,
    reason: bool,
}

fn literals(source: &str) -> Vec<Literal> {
    let chars: Vec<char> = source.chars().collect();
    let mut line = 1;
    let line_of: Vec<usize> = chars
        .iter()
        .map(|c| {
            let at = line;
            if *c == '\n' {
                line += 1;
            }
            at
        })
        .collect();
    let mut found = Vec::new();
    let mut code = String::new();
    let mut attr: Option<usize> = None;
    let mut depth = 0usize;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        let token_start = i == 0 || !is_word(chars[i - 1]);
        if c == '/' && next == Some('/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if c == '/' && next == Some('*') {
            let mut nest = 0usize;
            while i < chars.len() {
                if chars[i] == '/' && chars.get(i + 1) == Some(&'*') {
                    nest += 1;
                    i += 2;
                } else if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                    nest -= 1;
                    i += 2;
                    if nest == 0 {
                        break;
                    }
                } else {
                    i += 1;
                }
            }
            continue;
        }
        if c == '\'' {
            if next == Some('\\') {
                i += 3;
                while i < chars.len() && chars[i] != '\'' {
                    i += 1;
                }
                i += 1;
            } else if chars.get(i + 2) == Some(&'\'') {
                i += 3;
            } else {
                i += 1;
            }
            code.push('\'');
            continue;
        }
        let prefix = usize::from(
            token_start && (c == 'b' || c == 'c') && matches!(next, Some('r') | Some('"')),
        );
        let raw = token_start && chars.get(i + prefix) == Some(&'r') && {
            let mut k = i + prefix + 1;
            while chars.get(k) == Some(&'#') {
                k += 1;
            }
            chars.get(k) == Some(&'"')
        };
        if raw {
            let mut k = i + prefix + 1;
            let mut hashes = 0;
            while chars[k] == '#' {
                hashes += 1;
                k += 1;
            }
            let open = k;
            let mut close = open + 1;
            let body_end = loop {
                if close >= chars.len() {
                    break chars.len();
                }
                if chars[close] == '"'
                    && (0..hashes).all(|h| chars.get(close + 1 + h) == Some(&'#'))
                {
                    break close;
                }
                close += 1;
            };
            found.push(Literal {
                line: line_of[open],
                body: chars[open + 1..body_end].iter().collect(),
                reason: attr.is_some() && reason_before(&code),
            });
            i = (body_end + 1 + hashes).min(chars.len());
            code.push('"');
            continue;
        }
        if c == '"' || (prefix == 1 && next == Some('"')) {
            let open = i + prefix;
            let mut k = open + 1;
            let mut body = String::new();
            while k < chars.len() && chars[k] != '"' {
                if chars[k] == '\\' {
                    body.push(if chars.get(k + 1) == Some(&'\n') {
                        '\n'
                    } else {
                        ' '
                    });
                    k += 2;
                } else {
                    body.push(chars[k]);
                    k += 1;
                }
            }
            let body_end = k.min(chars.len());
            found.push(Literal {
                line: line_of[open],
                body,
                reason: attr.is_some() && reason_before(&code),
            });
            i = body_end + 1;
            code.push('"');
            continue;
        }
        if c == '#' && (next == Some('[') || (next == Some('!') && chars.get(i + 2) == Some(&'[')))
        {
            if attr.is_none() {
                attr = Some(depth);
            }
        } else if c == '[' {
            depth += 1;
        } else if c == ']' {
            depth = depth.saturating_sub(1);
            if attr == Some(depth) {
                attr = None;
            }
        }
        if !c.is_whitespace() {
            code.push(c);
        }
        i += 1;
    }
    found
}

fn reason_before(code: &str) -> bool {
    code.ends_with("ignore=")
}

fn scan_rust(path: &str, source: &str) -> Vec<Finding> {
    literals(source)
        .into_iter()
        .filter(|literal| !literal.reason)
        .flat_map(|literal| {
            literal
                .body
                .lines()
                .enumerate()
                .filter(|(_, part)| !word_hits(part).is_empty())
                .map(|(at, part)| Finding {
                    path: path.to_string(),
                    line: literal.line + at,
                    text: part.trim().to_string(),
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

fn listed(root: &Path) -> Result<Vec<String>, String> {
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
        .map_err(|e| format!("git ls-files: {e}"))?;
    if !out.status.success() {
        return Err("git ls-files failed".to_string());
    }
    let mut paths: Vec<String> = out
        .stdout
        .split(|b| *b == 0)
        .filter(|p| !p.is_empty())
        .map(|p| String::from_utf8_lossy(p).into_owned())
        .collect();
    paths.sort();
    paths.dedup();
    Ok(paths)
}

fn private_files(root: &Path) -> Vec<String> {
    let mut paths: Vec<String> = PRIVATE_FILES
        .iter()
        .filter(|name| root.join(name).is_file())
        .map(|name| name.to_string())
        .collect();
    let dev = root.join("dev");
    if dev.is_dir() {
        let mut stack = vec![dev];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.is_file()
                    && let Ok(relative) = path.strip_prefix(root)
                {
                    paths.push(relative.to_string_lossy().replace('\\', "/"));
                }
            }
        }
    }
    paths.sort();
    paths
}

fn script_like(path: &str) -> bool {
    path.starts_with("bin/") || path.starts_with(".config/") || path.ends_with(".sh")
}

fn rust_source(path: &str) -> bool {
    path.ends_with(".rs") && RUST_ROOTS.iter().any(|root| path.starts_with(root))
}

fn check(root: &Path) -> Result<Vec<Finding>, String> {
    let mut findings = Vec::new();
    let private = private_files(root);
    let mut paths = listed(root)?;
    paths.extend(private.iter().cloned());
    paths.sort();
    paths.dedup();
    for path in &paths {
        let rust = rust_source(path);
        let script = script_like(path) || private.contains(path);
        if !rust && !script {
            continue;
        }
        let Ok(text) = fs::read_to_string(root.join(path)) else {
            continue;
        };
        if rust {
            findings.extend(scan_rust(path, &text));
        } else {
            findings.extend(scan_text(path, &text));
        }
    }
    Ok(findings)
}

fn report(findings: &[Finding]) -> String {
    findings
        .iter()
        .map(|f| format!("{}:{}: {}", f.path, f.line, f.text))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn the_old_queue_command_never_returns() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let findings = check(&root).unwrap();
    assert!(
        findings.is_empty(),
        "the GPU queue is `pci gpu`; these lines name the old command:\n{}",
        report(&findings)
    );
}

struct Scratch {
    root: PathBuf,
}

impl Scratch {
    fn new(name: &str) -> Scratch {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tmp")
            .join(format!("queue-tool-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let ok = Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["init", "-q"])
            .status()
            .unwrap()
            .success();
        assert!(ok);
        Scratch { root }
    }

    fn write(&self, path: &str, text: &str) {
        let path = self.root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text.replace('@', OLD)).unwrap();
    }

    fn findings(&self) -> Vec<Finding> {
        check(&self.root).unwrap()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn at(path: &str, line: usize) -> (String, usize) {
    (path.to_string(), line)
}

fn places(findings: &[Finding]) -> Vec<(String, usize)> {
    findings.iter().map(|f| (f.path.clone(), f.line)).collect()
}

#[test]
fn a_script_that_runs_the_old_queue_fails_with_its_file_and_line() {
    let tree = Scratch::new("script");
    tree.write(
        "bin/render",
        "#!/bin/sh\nset -e\n@ run --class clip --as pfx -- pfx render\n",
    );
    tree.write("tools/shot.sh", "echo start\n@ turn\n");
    tree.write("notes/plain.txt", "@ run is not read here\n");
    let findings = tree.findings();
    assert_eq!(
        places(&findings),
        [at("bin/render", 3), at("tools/shot.sh", 2)]
    );
    let message = report(&findings);
    assert!(message.contains("bin/render:3: "), "{message}");
    assert!(
        message.contains(&format!("{OLD} run --class clip")),
        "{message}"
    );
}

#[test]
fn a_rule_file_that_sends_work_under_the_old_queue_fails() {
    let tree = Scratch::new("rules");
    tree.write("AGENTS.md", "# rules\n\nGPU tests run under @.\n");
    tree.write("dev/briefs/one.md", "first\nsecond\nqueue with `@ run`\n");
    tree.write(".config/queue.toml", "tool = \"@\"\n");
    assert_eq!(
        places(&tree.findings()),
        [
            at(".config/queue.toml", 1),
            at("AGENTS.md", 3),
            at("dev/briefs/one.md", 3)
        ]
    );
}

#[test]
fn a_public_clone_without_the_private_links_passes_quietly() {
    let tree = Scratch::new("public");
    tree.write(
        "bin/gate",
        "#!/bin/sh\npci gpu run --class clip -- cargo test\n",
    );
    tree.write(
        "src/lib.rs",
        "pub fn queue() -> &'static str {\n    \"pci\"\n}\n",
    );
    assert_eq!(tree.findings(), []);
}

#[test]
fn a_rust_string_that_names_the_old_queue_fails() {
    let tree = Scratch::new("rust");
    tree.write(
        "src/spawn.rs",
        "fn spawn() {\n    let _ = std::process::Command::new(\"@\").status();\n}\n",
    );
    tree.write(
        "crates/a/src/lib.rs",
        "pub fn note() -> String {\n    format!(\"no GPU queue (@) on the PATH\")\n}\n",
    );
    tree.write(
        "tests/fake.rs",
        "#[test]\nfn fake() {\n    let dir = std::path::Path::new(\"tmp\");\n    let _ = dir.join(\"@\");\n}\n",
    );
    tree.write(
        "examples/raw.rs",
        "fn main() {\n    let _a = 'x';\n    let _q = '\\'';\n    let s = r#\"say \"hi\"\n@ run\"#;\n    let _ = s;\n}\n",
    );
    tree.write(
        "crates/b/src/escaped.rs",
        "pub const LINE: &str = \"say \\\"go\\\" then @ turn\";\n",
    );
    tree.write(
        "src/tool.rs",
        "fn tool(p: &std::path::Path) {\n    std::fs::write(p, \"#!/bin/sh\\n@ turn\\n\").unwrap();\n}\n",
    );
    assert_eq!(
        places(&tree.findings()),
        [
            at("crates/a/src/lib.rs", 2),
            at("crates/b/src/escaped.rs", 1),
            at("examples/raw.rs", 5),
            at("src/spawn.rs", 2),
            at("src/tool.rs", 2),
            at("tests/fake.rs", 4)
        ]
    );
}

#[test]
fn identifiers_paths_environment_names_and_ignore_reasons_stay_allowed() {
    let tree = Scratch::new("allowed");
    tree.write(
        "crates/a/src/lib.rs",
        concat!(
            "pub mod @;\n",
            "use crate::@::Pgpu;\n",
            "pub fn @_turn(state: Pgpu) -> bool {\n",
            "    let @ = state == Pgpu::Free;\n",
            "    let _ = @::fetch();\n",
            "    let _ = std::env::var(\"PGPU_CLASS\");\n",
            "    let _ = std::env::var(\"PFX_EDIT_PGPU\");\n",
            "    let _ = \"crates/editor/src/@.rs\";\n",
            "    let _ = \"agent/@out\";\n",
            "    let _ = \"@::Pgpu\";\n",
            "    let _ = concat!(\"p\", \"gpu\");\n",
            "    @\n",
            "}\n",
            "#[test]\n",
            "#[ignore = \"needs a GPU; run under @\"]\n",
            "fn gpu() {}\n",
            "#[test]\n",
            "#[ignore = \"GPU: @ run --class clip --as pfx -- cargo test -- --ignored\"]\n",
            "fn clip() {}\n",
            "#[test]\n",
            "#[cfg_attr(not(feature = \"gpu\"), ignore = \"run under @\")]\n",
            "fn maybe() {}\n",
            "#[test]\n",
            "#[ignore =\n    \"needs a GPU; run under @\"]\n",
            "fn wrapped() {}\n",
        ),
    );
    tree.write(
        "bin/gate",
        "#!/bin/sh\nexport PGPU_CLASS=clip\nPFX_EDIT_PGPU=off pfx edit\nls crates/editor/src/@.rs\ngit log agent/@out\n# mod @ stays\nuse @::Pgpu\n",
    );
    tree.write("HANDOVER.md", "`pfx_editor::@::PgpuPlay` lives in `@.rs`\n");
    assert_eq!(tree.findings(), []);
}

#[test]
fn an_attribute_does_not_cover_a_string_after_it_closes() {
    let tree = Scratch::new("attr");
    tree.write(
        "src/a.rs",
        "#[ignore = \"run under @\"]\nfn a() {\n    let ignore = \"@\";\n    let _ = ignore;\n}\n",
    );
    tree.write("src/b.rs", "#[cfg(test)]\nconst A: [&str; 1] = [\"@\"];\n");
    assert_eq!(
        places(&tree.findings()),
        [at("src/a.rs", 3), at("src/b.rs", 2)]
    );
}

#[test]
fn comments_and_files_outside_the_scope_are_not_read() {
    let tree = Scratch::new("scope");
    tree.write("src/a.rs", "// @ run\n/* @ turn /* @ */ */\nfn a() {}\n");
    tree.write("docs/queue.md", "run it under @\n");
    tree.write("CHANGELOG.md", "- @ run learned a class\n");
    tree.write("other/lib.rs", "const A: &str = \"@\";\n");
    tree.write("src/data.toml", "tool = \"@\"\n");
    assert_eq!(tree.findings(), []);
}

#[test]
fn words_match_whole_and_lowercase_only() {
    assert_eq!(word_hits(&format!("{OLD} run")), [0]);
    assert_eq!(word_hits(&format!("({OLD})")), [1]);
    assert_eq!(word_hits(&format!("x {OLD}")), [2]);
    assert!(word_hits(&format!("{OLD}out")).is_empty());
    assert!(word_hits(&format!("x{OLD}")).is_empty());
    assert!(word_hits(&format!("{OLD}_turn")).is_empty());
    assert!(word_hits(&OLD.to_uppercase()).is_empty());
    assert!(word_hits("Pgpu").is_empty());
    assert!(word_hits(&format!("mod {OLD}")).is_empty());
    assert!(word_hits(&format!("{OLD}.rs")).is_empty());
    assert!(word_hits(&format!("{OLD}::Status")).is_empty());
    assert_eq!(word_hits(&format!("{OLD}.log")), [0]);
}
