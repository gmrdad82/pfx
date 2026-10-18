const IDS: &[&str] = &[
    "0BSD",
    "AGPL-3.0-only",
    "AGPL-3.0-or-later",
    "Apache-2.0",
    "Artistic-2.0",
    "BSD-2-Clause",
    "BSD-3-Clause",
    "BSL-1.0",
    "CC-BY-1.0",
    "CC-BY-2.0",
    "CC-BY-2.5",
    "CC-BY-3.0",
    "CC-BY-4.0",
    "CC-BY-NC-1.0",
    "CC-BY-NC-2.0",
    "CC-BY-NC-2.5",
    "CC-BY-NC-3.0",
    "CC-BY-NC-4.0",
    "CC-BY-NC-ND-1.0",
    "CC-BY-NC-ND-2.0",
    "CC-BY-NC-ND-2.5",
    "CC-BY-NC-ND-3.0",
    "CC-BY-NC-ND-4.0",
    "CC-BY-NC-SA-1.0",
    "CC-BY-NC-SA-2.0",
    "CC-BY-NC-SA-2.5",
    "CC-BY-NC-SA-3.0",
    "CC-BY-NC-SA-4.0",
    "CC-BY-ND-1.0",
    "CC-BY-ND-2.0",
    "CC-BY-ND-2.5",
    "CC-BY-ND-3.0",
    "CC-BY-ND-4.0",
    "CC-BY-SA-1.0",
    "CC-BY-SA-2.0",
    "CC-BY-SA-2.5",
    "CC-BY-SA-3.0",
    "CC-BY-SA-4.0",
    "CC-PDDC",
    "CC0-1.0",
    "CDLA-Permissive-2.0",
    "EPL-2.0",
    "FTL",
    "GPL-2.0-only",
    "GPL-2.0-or-later",
    "GPL-3.0-only",
    "GPL-3.0-or-later",
    "ISC",
    "LGPL-2.1-only",
    "LGPL-2.1-or-later",
    "LGPL-3.0-only",
    "LGPL-3.0-or-later",
    "Libpng",
    "MIT",
    "MIT-0",
    "MPL-2.0",
    "OFL-1.1",
    "Unlicense",
    "WTFPL",
    "Zlib",
];

fn custom(id: &str) -> bool {
    id.strip_prefix("LicenseRef-").is_some_and(|rest| {
        !rest.is_empty()
            && rest
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
    })
}

pub fn known(id: &str) -> bool {
    custom(id) || IDS.iter().any(|known| known.eq_ignore_ascii_case(id))
}

pub fn validate(expression: &str) -> Result<(), String> {
    let mut words = expression.split_whitespace();
    let mut expect_id = true;
    let mut seen = false;
    for word in words.by_ref() {
        if expect_id {
            if !known(word) {
                return Err(format!("unknown SPDX licence id `{word}`"));
            }
            seen = true;
        } else if word != "OR" && word != "AND" {
            return Err(format!(
                "expected OR or AND after a licence id, found `{word}`"
            ));
        }
        expect_id = !expect_id;
    }
    if !seen {
        return Err("no SPDX licence id".into());
    }
    if expect_id {
        return Err("the expression ends after OR or AND".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_and_flat_expressions_validate() {
        for ok in [
            "CC0-1.0",
            "cc-by-4.0",
            "MIT OR Apache-2.0",
            "CC-BY-4.0 AND OFL-1.1",
            "LicenseRef-Purchased-2026",
        ] {
            assert!(validate(ok).is_ok(), "{ok}");
        }
    }

    #[test]
    fn unknown_ids_are_refused_by_name() {
        assert!(validate("Creative").unwrap_err().contains("`Creative`"));
        assert!(
            validate("MIT OR Nope-1.0")
                .unwrap_err()
                .contains("`Nope-1.0`")
        );
        assert!(validate("(MIT)").unwrap_err().contains("`(MIT)`"));
        for bad in ["", "MIT OR", "MIT Apache-2.0", "LicenseRef-"] {
            assert!(validate(bad).is_err(), "{bad:?}");
        }
    }
}
