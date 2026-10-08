use std::collections::BTreeMap;
use std::sync::{LazyLock, PoisonError, RwLock};

static BOUND: LazyLock<RwLock<BTreeMap<String, String>>> = LazyLock::new(RwLock::default);

pub fn bind(key: &str, value: &str) {
    BOUND
        .write()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(key.to_string(), value.to_string());
}

pub fn bound(key: &str) -> Option<String> {
    BOUND
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .get(key)
        .cloned()
}

fn is_key(key: &str) -> bool {
    !key.is_empty()
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
}

pub fn filled(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let value = after
            .find('}')
            .map(|close| &after[..close])
            .filter(|key| is_key(key))
            .and_then(|key| bound(key).map(|value| (key.len(), value)));
        match value {
            Some((len, value)) => {
                out.push_str(&value);
                rest = &after[len + 1..];
            }
            None => {
                out.push('{');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bound_key_fills_its_token_and_anything_else_stays_as_written() {
        bind("bind-test-version", "1.4.2");
        bind("bind-test-name", "sample");
        assert_eq!(filled("v{bind-test-version}"), "v1.4.2");
        assert_eq!(
            filled("{bind-test-name} {bind-test-version}!"),
            "sample 1.4.2!"
        );
        assert_eq!(filled("{bind-test-unbound}"), "{bind-test-unbound}");
        assert_eq!(filled("{ bind-test-name }"), "{ bind-test-name }");
        assert_eq!(filled("{{bind-test-name}}"), "{sample}");
        assert_eq!(filled("{bind-test-name"), "{bind-test-name");
        assert_eq!(filled("}{}{"), "}{}{");
        assert_eq!(filled("plain"), "plain");
        bind("bind-test-name", "other");
        assert_eq!(filled("{bind-test-name}"), "other");
    }
}
