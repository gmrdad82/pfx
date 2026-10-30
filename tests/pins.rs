const PAGES: [(&str, &str); 3] = [
    ("README.md", include_str!("../README.md")),
    ("docs/first-game.md", include_str!("../docs/first-game.md")),
    ("docs/migrate-3.md", include_str!("../docs/migrate-3.md")),
];

fn pin(rest: &str) -> &str {
    rest.split(|c: char| !(c.is_ascii_alphanumeric() || c == '.' || c == '-'))
        .next()
        .unwrap_or_default()
}

#[test]
fn every_page_pins_pfx_at_the_workspace_version() {
    let tag = format!("v{}", env!("CARGO_PKG_VERSION"));
    let mut seen = 0;
    for (page, text) in PAGES {
        for line in text.lines().filter(|line| {
            line.contains("github.com/gmrdad82/pfx ")
                || line.contains("github.com/gmrdad82/pfx\"")
                || line.contains("github.com/gmrdad82/pfx.git")
        }) {
            let pins = line
                .match_indices("--tag v")
                .map(|(i, _)| pin(&line[i + 6..]))
                .chain(
                    line.match_indices("tag = \"v")
                        .map(|(i, _)| pin(&line[i + 7..])),
                );
            for found in pins {
                seen += 1;
                assert_eq!(
                    found, tag,
                    "{page} pins pfx at {found}; Cargo.toml is {tag}"
                );
            }
        }
    }
    assert!(seen > 0);
}

#[test]
fn the_key_model_lives_in_pfx_keys_and_its_origin_is_recorded() {
    let manifest = include_str!("../crates/keys/Cargo.toml");
    let dependencies: Vec<&str> = manifest
        .split("[dependencies]")
        .nth(1)
        .and_then(|rest| rest.split("\n[").next())
        .unwrap()
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    assert_eq!(dependencies.len(), 3, "{dependencies:?}");
    for (line, name) in dependencies.iter().zip(["crossterm", "egui", "winit"]) {
        assert!(line.starts_with(&format!("{name} = ")), "{line}");
        assert!(line.contains("optional = true"), "{line}");
        assert!(!line.contains("git ="), "{line}");
    }
    let tag = "v0.1.1";
    let commit = "c4554813b42b30a8764d80aef555f9bb8c5deffa";
    let record = include_str!("../crates/keys/SOURCE");
    let notice = include_str!("../NOTICE.md");
    for (file, text) in [("crates/keys/SOURCE", record), ("NOTICE.md", notice)] {
        assert!(text.contains(tag), "{file} lacks the model's tag {tag}");
        assert!(
            text.contains(commit),
            "{file} lacks the model's commit {commit}"
        );
    }
    for file in ["chord.rs", "keymap.rs", "sheet.rs", "terminal.rs"] {
        assert!(record.contains(file), "crates/keys/SOURCE lacks {file}");
    }
}
