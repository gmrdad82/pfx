use std::process::Command;

const ALLOWED: [&str; 14] = [
    "MIT",
    "Apache-2.0",
    "BSD-2-Clause",
    "BSD-3-Clause",
    "ISC",
    "Zlib",
    "Unicode-3.0",
    "Unicode-DFS-2016",
    "BSL-1.0",
    "CC0-1.0",
    "Unlicense",
    "0BSD",
    "CDLA-Permissive-2.0",
    "MIT-0",
];

struct Expression<'a> {
    tokens: Vec<&'a str>,
    at: usize,
}

impl<'a> Expression<'a> {
    fn new(text: &'a str) -> Self {
        let tokens = text
            .split_inclusive(['(', ')', ' '])
            .flat_map(|piece| {
                let word = piece.trim_end_matches(['(', ')', ' ']);
                let mark = piece[word.len()..].trim();
                [word, mark]
            })
            .filter(|token| !token.is_empty())
            .collect();
        Self { tokens, at: 0 }
    }

    fn next(&mut self) -> Option<&'a str> {
        let token = self.tokens.get(self.at).copied();
        self.at += 1;
        token
    }

    fn peek(&self) -> Option<&'a str> {
        self.tokens.get(self.at).copied()
    }

    fn any(&mut self) -> Result<bool, String> {
        let mut ok = self.all()?;
        while matches!(self.peek(), Some("OR" | "or" | "/")) {
            self.next();
            ok |= self.all()?;
        }
        Ok(ok)
    }

    fn all(&mut self) -> Result<bool, String> {
        let mut ok = self.one()?;
        while matches!(self.peek(), Some("AND" | "and")) {
            self.next();
            ok &= self.one()?;
        }
        Ok(ok)
    }

    fn one(&mut self) -> Result<bool, String> {
        match self.next() {
            Some("(") => {
                let ok = self.any()?;
                match self.next() {
                    Some(")") => Ok(ok),
                    other => Err(format!("expected ')', found {other:?}")),
                }
            }
            Some(id) => {
                if self.peek() == Some("WITH") {
                    self.next();
                    self.next();
                }
                Ok(ALLOWED.contains(&id))
            }
            None => Err("an empty expression".into()),
        }
    }
}

fn allowed(licence: &str) -> Result<bool, String> {
    let spaced = licence.replace('/', " / ");
    let mut expression = Expression::new(&spaced);
    let ok = expression.any()?;
    match expression.peek() {
        None => Ok(ok),
        Some(rest) => Err(format!("unexpected {rest:?}")),
    }
}

#[test]
fn every_dependency_is_under_a_licence_the_mit_code_can_ship_with() {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let output = Command::new(cargo)
        .args(["metadata", "--format-version", "1", "--locked"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let mut refused = Vec::new();
    for package in metadata["packages"].as_array().unwrap() {
        let name = package["name"].as_str().unwrap_or("?");
        let version = package["version"].as_str().unwrap_or("?");
        match package["license"].as_str() {
            Some(licence) => match allowed(licence) {
                Ok(true) => {}
                Ok(false) => refused.push(format!("{name} {version}: {licence}")),
                Err(error) => refused.push(format!("{name} {version}: {licence} ({error})")),
            },
            None => refused.push(format!("{name} {version}: no licence field")),
        }
    }
    assert!(
        refused.is_empty(),
        "dependencies outside the allowed licences:\n{}",
        refused.join("\n")
    );
}

#[test]
fn the_expression_reader_weighs_and_over_or() {
    assert_eq!(allowed("MIT OR Apache-2.0"), Ok(true));
    assert_eq!(allowed("Apache-2.0 OR GPL-2.0-only"), Ok(true));
    assert_eq!(allowed("MIT/Apache-2.0"), Ok(true));
    assert_eq!(allowed("Apache-2.0 WITH LLVM-exception"), Ok(true));
    assert_eq!(allowed("(MIT OR Apache-2.0) AND Unicode-3.0"), Ok(true));
    assert_eq!(allowed("GPL-3.0-only"), Ok(false));
    assert_eq!(allowed("(MIT OR Apache-2.0) AND GPL-3.0-only"), Ok(false));
    assert_eq!(allowed("MIT AND LGPL-2.1-or-later"), Ok(false));
}
