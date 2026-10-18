use std::collections::BTreeSet;

pub const EDITORIAL: &str = "LicenseRef-Editorial";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    NonCommercial,
    NoDerivatives,
    Editorial,
    Custom,
}

impl Kind {
    pub const ALL: [Kind; 4] = [
        Kind::NonCommercial,
        Kind::NoDerivatives,
        Kind::Editorial,
        Kind::Custom,
    ];

    pub fn word(self) -> &'static str {
        match self {
            Kind::NonCommercial => "nc",
            Kind::NoDerivatives => "nd",
            Kind::Editorial => "editorial",
            Kind::Custom => "custom",
        }
    }

    pub fn parse(word: &str) -> Result<Kind, String> {
        Kind::ALL
            .into_iter()
            .find(|kind| kind.word().eq_ignore_ascii_case(word.trim()))
            .ok_or_else(|| {
                format!(
                    "unknown licence kind `{word}`, expected one of: {}",
                    Kind::ALL.map(Kind::word).join(", ")
                )
            })
    }

    fn meaning(self) -> &'static str {
        match self {
            Kind::NonCommercial => "non-commercial only",
            Kind::NoDerivatives => "no derivatives",
            Kind::Editorial => "editorial use only",
            Kind::Custom => "a custom licence",
        }
    }
}

pub fn kinds(id: &str) -> BTreeSet<Kind> {
    let upper = id.to_ascii_uppercase();
    let mut out = BTreeSet::new();
    if upper.eq_ignore_ascii_case(EDITORIAL) {
        out.insert(Kind::Editorial);
    } else if upper.starts_with("LICENSEREF-") {
        out.insert(Kind::Custom);
    }
    let words: Vec<&str> = upper.split('-').collect();
    if words.contains(&"NC") {
        out.insert(Kind::NonCommercial);
    }
    if words.contains(&"ND") {
        out.insert(Kind::NoDerivatives);
    }
    out
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Policy {
    pub allow: BTreeSet<Kind>,
    pub only: Vec<String>,
}

impl Policy {
    pub fn with(&self, extra: &[Kind]) -> Policy {
        let mut out = self.clone();
        out.allow.extend(extra.iter().copied());
        out
    }

    fn id_problem(&self, id: &str) -> Option<String> {
        if !self.only.is_empty() && !self.only.iter().any(|o| o.eq_ignore_ascii_case(id)) {
            return Some(format!(
                "{id} is not among the licences allowed in pfx's settings ({})",
                self.only.join(", ")
            ));
        }
        let missing: Vec<Kind> = kinds(id)
            .into_iter()
            .filter(|kind| !self.allow.contains(kind))
            .collect();
        if missing.is_empty() {
            return None;
        }
        let meaning: Vec<&str> = missing.iter().map(|k| k.meaning()).collect();
        let flags: Vec<String> = missing
            .iter()
            .map(|k| format!("--allow {}", k.word()))
            .collect();
        Some(format!(
            "{id} is {}, refused unless {}",
            meaning.join(" and "),
            flags.join(" ")
        ))
    }

    pub fn check(&self, expression: &str) -> Result<(), String> {
        let expression = expression.trim();
        if expression.is_empty() {
            return Err("no licence: an asset without an SPDX licence id is refused".into());
        }
        pfx_core::spdx::validate(expression)?;
        let mut problems = Vec::new();
        for alternative in expression.split(" OR ") {
            let found: Vec<String> = alternative
                .split(" AND ")
                .filter_map(|id| self.id_problem(id.trim()))
                .collect();
            if found.is_empty() {
                return Ok(());
            }
            problems.extend(found);
        }
        Err(problems.join("; "))
    }
}

pub fn label(id: &str) -> String {
    let upper = id.to_ascii_uppercase();
    if let Some(rest) = upper.strip_prefix("CC-") {
        if let Some((name, version)) = rest.rsplit_once('-')
            && version.contains('.')
        {
            return format!("CC {name} {version}");
        }
        return format!("CC {rest}");
    }
    if let Some(version) = upper.strip_prefix("CC0-") {
        return format!("CC0 {version}");
    }
    id.to_string()
}

pub fn credit_line(title: &str, author: Option<&str>, url: Option<&str>, licence: &str) -> String {
    let mut line = format!("\"{title}\"");
    if let Some(author) = author.filter(|a| !a.is_empty()) {
        line.push_str(&format!(" by {author}"));
    }
    if let Some(url) = url.filter(|u| !u.is_empty()) {
        line.push_str(&format!(" ({url})"));
    }
    let named: Vec<String> = licence
        .split_whitespace()
        .map(|word| match word {
            "OR" => "or".to_string(),
            "AND" => "and".to_string(),
            id => label(id),
        })
        .collect();
    line.push_str(&format!(" is licensed under {}", named.join(" ")));
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nc_nd_and_editorial_are_refused_until_allowed() {
        let strict = Policy::default();
        assert!(strict.check("CC0-1.0").is_ok());
        assert!(strict.check("CC-BY-4.0").is_ok());
        assert!(strict.check("CC-BY-SA-4.0").is_ok());
        let nc = strict.check("CC-BY-NC-4.0").unwrap_err();
        assert!(nc.contains("--allow nc"), "{nc}");
        let ncnd = strict.check("CC-BY-NC-ND-4.0").unwrap_err();
        assert!(ncnd.ends_with("--allow nc --allow nd"), "{ncnd}");
        assert!(
            strict
                .check(EDITORIAL)
                .unwrap_err()
                .contains("--allow editorial")
        );
        assert!(
            strict
                .check("LicenseRef-Store")
                .unwrap_err()
                .contains("--allow custom")
        );
        assert!(
            strict
                .with(&[Kind::NonCommercial])
                .check("CC-BY-NC-4.0")
                .is_ok()
        );
        assert!(
            strict
                .with(&[Kind::NonCommercial])
                .check("CC-BY-NC-ND-4.0")
                .is_err()
        );
        assert!(
            strict
                .with(&[Kind::NonCommercial, Kind::NoDerivatives])
                .check("CC-BY-NC-ND-4.0")
                .is_ok()
        );
        assert!(strict.with(&[Kind::Editorial]).check(EDITORIAL).is_ok());
    }

    #[test]
    fn an_or_takes_any_allowed_side_and_an_and_needs_every_id() {
        let strict = Policy::default();
        assert!(strict.check("CC-BY-NC-4.0 OR CC0-1.0").is_ok());
        assert!(strict.check("CC0-1.0 AND CC-BY-ND-4.0").is_err());
        assert!(strict.check("").is_err());
        assert!(
            strict
                .check("Free-Stuff")
                .unwrap_err()
                .contains("`Free-Stuff`")
        );
    }

    #[test]
    fn the_settings_list_narrows_what_is_taken() {
        let only = Policy {
            allow: BTreeSet::new(),
            only: vec!["CC0-1.0".into()],
        };
        assert!(only.check("cc0-1.0").is_ok());
        assert!(only.check("CC-BY-4.0").unwrap_err().contains("settings"));
    }

    #[test]
    fn kinds_parse_by_word() {
        for kind in Kind::ALL {
            assert_eq!(Kind::parse(kind.word()), Ok(kind));
        }
        assert!(Kind::parse("sa").is_err());
    }

    #[test]
    fn credit_lines_name_title_author_source_and_licence() {
        assert_eq!(
            credit_line(
                "Rock Wall",
                Some("A. Author"),
                Some("https://example.org/a/rock"),
                "CC-BY-4.0"
            ),
            "\"Rock Wall\" by A. Author (https://example.org/a/rock) is licensed under CC BY 4.0"
        );
        assert_eq!(
            credit_line("Sky", None, None, "CC0-1.0"),
            "\"Sky\" is licensed under CC0 1.0"
        );
        assert_eq!(label("CC-BY-NC-SA-4.0"), "CC BY-NC-SA 4.0");
        assert_eq!(
            credit_line("Pair", None, None, "MIT OR Apache-2.0"),
            "\"Pair\" is licensed under MIT or Apache-2.0"
        );
    }
}
