#![cfg(feature = "harness")]

use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const UPDATE_HINT: &str = "tests/agnostic/update.sh";
const SKIPPED_FILES: [&str; 4] = ["CONVENTIONS.md", "HANDOVER.md", "ROADMAP.md", "AGENTS.md"];
const SKIPPED_PREFIXES: [&str; 2] = ["prompts/", "tests/agnostic/"];
const ASSET_EXTENSIONS: [&str; 30] = [
    "ttf", "otf", "ttc", "woff", "woff2", "png", "jpg", "jpeg", "gif", "webp", "bmp", "tga", "exr",
    "hdr", "ico", "avif", "ktx", "ktx2", "tif", "tiff", "svg", "wav", "ogg", "mp3", "flac", "opus",
    "aif", "aiff", "m4a", "oga",
];
const SOURCE_KINDS: [&str; 4] = ["generated:", "OFL:", "CC0:", "product:"];

type Counts = BTreeMap<String, BTreeMap<String, usize>>;

#[derive(Clone, Copy)]
enum Kind {
    Word,
    Text,
    Hex,
}

struct Rule {
    label: String,
    kind: Kind,
    pattern: String,
}

struct Deny {
    rules: Vec<Rule>,
    allow: Vec<String>,
}

#[derive(Default)]
struct Report {
    failures: Vec<String>,
    notes: Vec<String>,
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn parse_deny(text: &str) -> Result<Deny, String> {
    let mut rules = Vec::new();
    let mut allow = Vec::new();
    for (number, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (first, rest) = line
            .split_once(char::is_whitespace)
            .ok_or_else(|| format!("deny.txt line {}: expected fields", number + 1))?;
        let rest = rest.trim();
        if first == "allow" {
            allow.push(rest.to_lowercase());
            continue;
        }
        let (kind, pattern) = rest
            .split_once(char::is_whitespace)
            .ok_or_else(|| format!("deny.txt line {}: expected a pattern", number + 1))?;
        let kind = match kind {
            "word" => Kind::Word,
            "text" => Kind::Text,
            "hex" => Kind::Hex,
            other => {
                return Err(format!(
                    "deny.txt line {}: unknown kind {other}",
                    number + 1
                ));
            }
        };
        rules.push(Rule {
            label: first.to_string(),
            kind,
            pattern: pattern.trim().to_lowercase(),
        });
    }
    Ok(Deny { rules, allow })
}

fn hex_run(text: &str) -> usize {
    text.chars().take_while(char::is_ascii_hexdigit).count()
}

fn count_matches(line: &str, rule: &Rule) -> usize {
    let mut found = 0;
    for (at, _) in line.match_indices(&rule.pattern) {
        let before = line[..at].chars().next_back();
        let after_text = &line[at + rule.pattern.len()..];
        let after = after_text.chars().next();
        let fits = match rule.kind {
            Kind::Text => true,
            Kind::Word => !before.is_some_and(is_word) && !after.is_some_and(is_word),
            Kind::Hex => {
                !before.is_some_and(|c| c.is_ascii_hexdigit())
                    && matches!(hex_run(after_text), 0 | 2)
                    && !after_text
                        .chars()
                        .nth(hex_run(after_text))
                        .is_some_and(is_word)
            }
        };
        if fits {
            found += 1;
        }
    }
    found
}

fn count_line(deny: &Deny, line: &str, into: &mut BTreeMap<String, usize>) {
    let mut text = line.to_lowercase();
    for phrase in &deny.allow {
        if text.contains(phrase.as_str()) {
            text = text.replace(phrase.as_str(), " ");
        }
    }
    for rule in &deny.rules {
        let n = count_matches(&text, rule);
        if n > 0 {
            *into.entry(rule.label.clone()).or_insert(0) += n;
        }
    }
}

fn tracked(root: &Path) -> Result<Vec<String>, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["ls-files", "-z"])
        .output()
        .map_err(|e| format!("git ls-files: {e}"))?;
    if !out.status.success() {
        return Err("git ls-files failed".to_string());
    }
    Ok(out
        .stdout
        .split(|b| *b == 0)
        .filter(|p| !p.is_empty())
        .map(|p| String::from_utf8_lossy(p).into_owned())
        .collect())
}

fn skipped(path: &str) -> bool {
    SKIPPED_FILES.contains(&path) || SKIPPED_PREFIXES.iter().any(|p| path.starts_with(p))
}

fn scan(root: &Path, deny: &Deny) -> Result<Counts, String> {
    let mut all = Counts::new();
    for path in tracked(root)? {
        if skipped(&path) {
            continue;
        }
        let mut counts = BTreeMap::new();
        count_line(deny, &path, &mut counts);
        if let Ok(bytes) = fs::read(root.join(&path)) {
            let head = &bytes[..bytes.len().min(8192)];
            if !head.contains(&0) {
                for line in String::from_utf8_lossy(&bytes).lines() {
                    count_line(deny, line, &mut counts);
                }
            }
        }
        if !counts.is_empty() {
            all.insert(path, counts);
        }
    }
    Ok(all)
}

fn format_baseline(counts: &Counts) -> String {
    let mut out = String::from(
        "# generated by tests/agnostic/update.sh: one line per tracked file with hits, `label=count` each.\n# It may only shrink; a rise or a new file fails the check.\n",
    );
    for (path, labels) in counts {
        let fields: Vec<String> = labels.iter().map(|(l, n)| format!("{l}={n}")).collect();
        out.push_str(&format!("{path}\t{}\n", fields.join(" ")));
    }
    out
}

fn parse_baseline(text: &str) -> Result<Counts, String> {
    let mut counts = Counts::new();
    for (number, line) in text.lines().enumerate() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (path, fields) = line
            .split_once('\t')
            .ok_or_else(|| format!("baseline.txt line {}: expected a tab", number + 1))?;
        let mut labels = BTreeMap::new();
        for field in fields.split_whitespace() {
            let (label, n) = field
                .split_once('=')
                .ok_or_else(|| format!("baseline.txt line {}: bad field {field}", number + 1))?;
            let n = n
                .parse()
                .map_err(|_| format!("baseline.txt line {}: bad count {field}", number + 1))?;
            labels.insert(label.to_string(), n);
        }
        counts.insert(path.to_string(), labels);
    }
    Ok(counts)
}

fn read_in(root: &Path, name: &str) -> Result<String, String> {
    let path = root.join("tests").join("agnostic").join(name);
    fs::read_to_string(&path).map_err(|e| format!("tests/agnostic/{name}: {e}"))
}

fn compare(current: &Counts, baseline: &Counts, report: &mut Report) {
    for (path, labels) in current {
        for (label, n) in labels {
            let allowed = baseline
                .get(path)
                .and_then(|l| l.get(label))
                .copied()
                .unwrap_or(0);
            if *n > allowed {
                report
                    .failures
                    .push(if allowed == 0 && !baseline.contains_key(path) {
                        format!("{path}: {n} hit(s) of `{label}` in a file outside the baseline")
                    } else {
                        format!("{path}: `{label}` rose from {allowed} to {n}")
                    });
            }
        }
    }
    let mut fell = 0;
    for (path, labels) in baseline {
        for (label, was) in labels {
            let now = current
                .get(path)
                .and_then(|l| l.get(label))
                .copied()
                .unwrap_or(0);
            if now < *was {
                fell += *was - now;
            }
        }
    }
    if fell > 0 {
        report.notes.push(format!(
            "the baseline can shrink by {fell} hit(s); rewrite it with: {UPDATE_HINT}"
        ));
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn is_asset(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| ASSET_EXTENSIONS.contains(&e.to_lowercase().as_str()))
}

struct Listed {
    sha: String,
    source: String,
}

fn parse_fixtures(text: &str) -> Result<BTreeMap<String, Listed>, String> {
    let mut listed = BTreeMap::new();
    for (number, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let bad = || {
            format!(
                "fixtures.txt line {}: expected `sha256  path  source`",
                number + 1
            )
        };
        let (sha, rest) = line.split_once(char::is_whitespace).ok_or_else(bad)?;
        let (path, source) = rest.trim().split_once("  ").ok_or_else(bad)?;
        let source = source.trim();
        if !SOURCE_KINDS.iter().any(|k| source.starts_with(k)) {
            return Err(format!(
                "fixtures.txt line {}: source must start with one of {}",
                number + 1,
                SOURCE_KINDS.join(" ")
            ));
        }
        listed.insert(
            path.trim().to_string(),
            Listed {
                sha: sha.to_string(),
                source: source.to_string(),
            },
        );
    }
    Ok(listed)
}

fn check_fixtures(root: &Path, report: &mut Report) -> Result<(), String> {
    let listed = parse_fixtures(&read_in(root, "fixtures.txt")?)?;
    let files = tracked(root)?;
    let mut seen = Vec::new();
    for path in files
        .iter()
        .filter(|p| is_asset(p) && !p.starts_with("prompts/"))
    {
        seen.push(path.clone());
        let Ok(bytes) = fs::read(root.join(path)) else {
            continue;
        };
        let sha = sha256_hex(&bytes);
        match listed.get(path) {
            None => report.failures.push(format!(
                "{path}: asset file not listed in tests/agnostic/fixtures.txt; add `{sha}  {path}  <generated: script | OFL: source | CC0: source>`"
            )),
            Some(entry) if entry.sha != sha => report.failures.push(format!(
                "{path}: changed (listed {}, now {sha}); update its line in tests/agnostic/fixtures.txt",
                entry.sha
            )),
            Some(_) => {}
        }
    }
    for (path, entry) in &listed {
        if !seen.contains(path) {
            report.notes.push(format!(
                "tests/agnostic/fixtures.txt lists {path} ({}) but it is no longer tracked; drop the line",
                entry.source
            ));
        }
    }
    Ok(())
}

fn check(root: &Path) -> Result<Report, String> {
    let deny = parse_deny(&read_in(root, "deny.txt")?)?;
    let baseline = parse_baseline(&read_in(root, "baseline.txt")?)?;
    let current = scan(root, &deny)?;
    let mut report = Report::default();
    compare(&current, &baseline, &mut report);
    check_fixtures(root, &mut report)?;
    Ok(report)
}

fn rewrite_baseline(root: &Path, allow_growth: bool) -> Result<(), String> {
    let deny = parse_deny(&read_in(root, "deny.txt")?)?;
    let current = scan(root, &deny)?;
    if !allow_growth {
        let baseline = parse_baseline(&read_in(root, "baseline.txt")?)?;
        let mut report = Report::default();
        compare(&current, &baseline, &mut report);
        if !report.failures.is_empty() {
            return Err(format!(
                "the baseline only shrinks; fix these first:\n{}",
                report.failures.join("\n")
            ));
        }
    }
    let path = root.join("tests").join("agnostic").join("baseline.txt");
    fs::write(&path, format_baseline(&current)).map_err(|e| format!("baseline.txt: {e}"))
}

fn engine_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn no_product_name_or_asset_enters_the_engine() {
    let report = check(&engine_root()).unwrap();
    for note in &report.notes {
        eprintln!("agnostic: {note}");
    }
    assert!(
        report.failures.is_empty(),
        "the engine is generic and agnostic (docs/agnostic.md):\n{}",
        report.failures.join("\n")
    );
}

#[test]
#[ignore = "rewrites tests/agnostic/baseline.txt; run through tests/agnostic/update.sh"]
fn rewrite_the_baseline() {
    let grow = std::env::var("AGNOSTIC_GROW").is_ok_and(|v| v == "1");
    rewrite_baseline(&engine_root(), grow).unwrap();
}

struct Scratch {
    root: PathBuf,
}

impl Scratch {
    fn new(name: &str) -> Scratch {
        let root = engine_root()
            .join("tmp")
            .join(format!("agnostic-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("tests").join("agnostic")).unwrap();
        let git = |args: &[&str]| {
            let ok = Command::new("git")
                .arg("-C")
                .arg(&root)
                .args(args)
                .status()
                .unwrap()
                .success();
            assert!(ok);
        };
        git(&["init", "-q"]);
        let scratch = Scratch { root };
        scratch.write(
            "tests/agnostic/deny.txt",
            "# test list\nallow  foo allowed\nfoo  word  foo\nbar  text  bar\ncol  hex  c0ffee\n",
        );
        scratch.write("tests/agnostic/baseline.txt", "");
        scratch.write("tests/agnostic/fixtures.txt", "");
        scratch
    }

    fn write(&self, path: &str, text: &str) {
        let full = self.root.join(path);
        fs::create_dir_all(full.parent().unwrap()).unwrap();
        fs::write(full, text).unwrap();
    }

    fn write_bytes(&self, path: &str, bytes: &[u8]) {
        let full = self.root.join(path);
        fs::create_dir_all(full.parent().unwrap()).unwrap();
        fs::write(full, bytes).unwrap();
    }

    fn add(&self) {
        let ok = Command::new("git")
            .arg("-C")
            .arg(&self.root)
            .args(["add", "-A"])
            .status()
            .unwrap()
            .success();
        assert!(ok);
    }

    fn baseline_from_now(&self) {
        self.add();
        rewrite_baseline(&self.root, true).unwrap();
    }

    fn report(&self) -> Report {
        self.add();
        check(&self.root).unwrap()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn a_clean_tree_passes_and_says_nothing() {
    let repo = Scratch::new("clean");
    repo.write("a.txt", "nothing here\n");
    repo.baseline_from_now();
    let report = repo.report();
    assert!(report.failures.is_empty());
    assert!(report.notes.is_empty());
}

#[test]
fn a_new_hit_in_a_file_outside_the_baseline_fails() {
    let repo = Scratch::new("new-hit");
    repo.write("a.txt", "foo\n");
    repo.baseline_from_now();
    repo.write("b.txt", "a foo here\n");
    let report = repo.report();
    assert_eq!(report.failures.len(), 1);
    assert!(report.failures[0].starts_with("b.txt"));
    assert!(report.failures[0].contains("outside the baseline"));
}

#[test]
fn a_new_label_in_a_baselined_file_fails() {
    let repo = Scratch::new("new-label");
    repo.write("a.txt", "foo\n");
    repo.baseline_from_now();
    repo.write("a.txt", "foo\nbar\n");
    let report = repo.report();
    assert_eq!(report.failures.len(), 1);
    assert!(report.failures[0].contains("`bar` rose from 0 to 1"));
}

#[test]
fn a_raised_count_fails() {
    let repo = Scratch::new("raised");
    repo.write("a.txt", "foo\nfoo\n");
    repo.baseline_from_now();
    repo.write("a.txt", "foo\nfoo\nfoo\n");
    let report = repo.report();
    assert_eq!(report.failures.len(), 1);
    assert!(report.failures[0].contains("rose from 2 to 3"));
}

#[test]
fn a_fall_passes_with_the_rewrite_hint_and_the_rewrite_takes_it() {
    let repo = Scratch::new("fall");
    repo.write("a.txt", "foo\nfoo\n");
    repo.write("b.txt", "bar\n");
    repo.baseline_from_now();
    repo.write("a.txt", "foo\n");
    fs::remove_file(repo.root.join("b.txt")).unwrap();
    let report = repo.report();
    assert!(report.failures.is_empty());
    assert_eq!(report.notes.len(), 1);
    assert!(report.notes[0].contains("can shrink by 2"));
    assert!(report.notes[0].contains(UPDATE_HINT));
    rewrite_baseline(&repo.root, false).unwrap();
    let after = repo.report();
    assert!(after.failures.is_empty());
    assert!(after.notes.is_empty());
    let text = read_in(&repo.root, "baseline.txt").unwrap();
    assert!(text.contains("a.txt\tfoo=1"));
    assert!(!text.contains("b.txt"));
}

#[test]
fn the_rewrite_refuses_to_grow_the_baseline() {
    let repo = Scratch::new("refuse");
    repo.write("a.txt", "foo\n");
    repo.baseline_from_now();
    repo.write("a.txt", "foo\nfoo\n");
    repo.add();
    let err = rewrite_baseline(&repo.root, false).unwrap_err();
    assert!(err.contains("only shrinks"));
    assert!(
        read_in(&repo.root, "baseline.txt")
            .unwrap()
            .contains("foo=1")
    );
}

#[test]
fn skipped_paths_are_not_read() {
    let repo = Scratch::new("skipped");
    repo.write("prompts/x.md", "foo\n");
    repo.write("ROADMAP.md", "foo\n");
    repo.write("HANDOVER.md", "foo\n");
    repo.write("CONVENTIONS.md", "foo\n");
    repo.write("AGENTS.md", "foo\n");
    repo.write("tests/agnostic/notes.txt", "foo\n");
    let report = repo.report();
    assert!(report.failures.is_empty());
}

#[test]
fn a_tracked_file_name_counts() {
    let repo = Scratch::new("names");
    repo.write("foo.txt", "plain\n");
    let report = repo.report();
    assert_eq!(report.failures.len(), 1);
    assert!(report.failures[0].starts_with("foo.txt"));
}

#[test]
fn word_text_hex_and_allow_rules_match_as_written() {
    let deny =
        parse_deny("allow  foo allowed\nfoo word foo\nbar text bar\ncol hex c0ffee\n").unwrap();
    let count = |line: &str| {
        let mut into = BTreeMap::new();
        count_line(&deny, line, &mut into);
        into
    };
    assert_eq!(count("Foo FOO"), BTreeMap::from([("foo".to_string(), 2)]));
    assert!(!count("foobar_foo foo_bar afoo").contains_key("foo"));
    assert_eq!(count("foo.bar(foo)")["foo"], 2);
    assert!(count("a foo allowed here").is_empty());
    assert_eq!(count("xbarx bar")["bar"], 2);
    assert_eq!(count("#C0FFEE")["col"], 1);
    assert_eq!(count("#c0ffeecc")["col"], 1);
    assert_eq!(count("0xc0ffee")["col"], 1);
    assert!(count("c0ffee4455").is_empty());
    assert!(count("9c0ffee").is_empty());
}

#[test]
fn an_unlisted_png_fails_and_a_listed_one_passes() {
    let repo = Scratch::new("png");
    let bytes = b"\x89PNG not really".to_vec();
    repo.write_bytes("tests/pic.png", &bytes);
    let report = repo.report();
    assert_eq!(report.failures.len(), 1);
    assert!(report.failures[0].contains("tests/pic.png"));
    assert!(report.failures[0].contains(&sha256_hex(&bytes)));
    repo.write(
        "tests/agnostic/fixtures.txt",
        &format!(
            "{}  tests/pic.png  generated: scripts/pic.sh\n",
            sha256_hex(&bytes)
        ),
    );
    let report = repo.report();
    assert!(report.failures.is_empty(), "{:?}", report.failures);
}

#[test]
fn a_changed_asset_fails_and_a_dropped_one_is_noted() {
    let repo = Scratch::new("changed");
    let bytes = b"RIFF one".to_vec();
    repo.write_bytes("docs/clip.wav", &bytes);
    repo.write(
        "tests/agnostic/fixtures.txt",
        &format!(
            "{}  docs/clip.wav  CC0: example\n{}  docs/gone.svg  OFL: example\n",
            sha256_hex(&bytes),
            sha256_hex(b"gone")
        ),
    );
    let report = repo.report();
    assert!(report.failures.is_empty());
    assert_eq!(report.notes.len(), 1);
    assert!(report.notes[0].contains("docs/gone.svg"));
    repo.write_bytes("docs/clip.wav", b"RIFF two");
    let report = repo.report();
    assert_eq!(report.failures.len(), 1);
    assert!(report.failures[0].contains("changed"));
}

#[test]
fn a_source_without_a_known_kind_is_refused() {
    assert!(parse_fixtures("abc  tests/a.png  somewhere\n").is_err());
    assert!(parse_fixtures("abc  tests/a.png  product: example, goes in wave D\n").is_ok());
}
