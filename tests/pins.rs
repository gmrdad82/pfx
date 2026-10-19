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
