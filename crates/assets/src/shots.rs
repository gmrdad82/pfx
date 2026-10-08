use serde_json::{Map, Value};

pub const SHOT_KEYS: &[&str] = &[
    "line", "seconds", "closure", "preset", "camera", "keys", "assets", "rig", "style", "outline",
    "exposure", "view", "look", "denoise", "daylight",
];
pub const SCENE_SECTIONS: &[&str] = &[
    "rig", "style", "outline", "exposure", "view", "look", "denoise",
];
pub const ENTRY_KEYS: &[&str] = &[
    "use", "at", "turn", "tilt", "scale", "count", "layout", "columns", "spacing", "jitter",
    "preset",
];
pub const KINDS: &[&str] = &["logo", "wordmark", "lockup", "tile", "icon"];

#[derive(Clone, Debug, PartialEq)]
pub enum Closure {
    Open,
    Palindrome,
    Crossfade(f64),
}

impl Closure {
    pub fn read(value: Option<&Value>) -> Result<Closure, String> {
        match value {
            None => Ok(Closure::Open),
            Some(Value::String(s)) if s == "palindrome" => Ok(Closure::Palindrome),
            Some(Value::Object(t)) => match t.get("crossfade").and_then(Value::as_f64) {
                Some(s) if s > 0.0 => Ok(Closure::Crossfade(s)),
                _ => Err("closure table needs crossfade = <seconds>".into()),
            },
            Some(other) => Err(format!(
                "closure {other} is neither \"palindrome\" nor {{ crossfade = <seconds> }}"
            )),
        }
    }

    pub fn label(&self) -> Value {
        match self {
            Closure::Open => Value::Null,
            Closure::Palindrome => Value::from("palindrome"),
            Closure::Crossfade(s) => serde_json::json!({ "crossfade": s }),
        }
    }
}

pub fn check_shot(name: &str, shot: &Map<String, Value>) -> Result<(), String> {
    let place = format!("shot {name}");
    if let Some(bad) = shot.keys().find(|k| !SHOT_KEYS.contains(&k.as_str())) {
        return Err(format!(
            "{place}: unknown key {bad}; allowed: {}",
            SHOT_KEYS.join(", ")
        ));
    }
    let assets = shot
        .get("assets")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{place}: needs at least one [[shots.{name}.assets]] entry"))?;
    if assets.is_empty() {
        return Err(format!("{place}: needs at least one asset"));
    }
    for (i, entry) in assets.iter().enumerate() {
        let entry = entry
            .as_object()
            .ok_or_else(|| format!("{place}: asset {i} is not a table"))?;
        if let Some(bad) = entry.keys().find(|k| !ENTRY_KEYS.contains(&k.as_str())) {
            return Err(format!(
                "{place}: asset {i} has unknown key {bad}; allowed: {}",
                ENTRY_KEYS.join(", ")
            ));
        }
        if entry.get("use").and_then(Value::as_str).is_none() {
            return Err(format!(
                "{place}: asset {i} needs use = \"<kind>:<product>[:<name>]\""
            ));
        }
    }
    if shot
        .get("seconds")
        .is_some_and(|v| !v.as_f64().is_some_and(|s| s > 0.0))
    {
        return Err(format!("{place}: seconds must be above 0"));
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq)]
pub struct Pick {
    pub kind: String,
    pub product: String,
    pub name: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Want {
    pub kind: String,
    pub product: String,
    pub set: String,
    pub icon: String,
}

pub fn parse_use(text: &str) -> Result<Want, String> {
    let parts: Vec<&str> = text.split(':').collect();
    let (kind, product, name) = match parts[..] {
        [k, p] => (k, p, None),
        [k, p, n] => (k, p, Some(n)),
        _ => return Err(format!("use is <kind>:<product>[:<name>]; got {text}")),
    };
    if !KINDS.contains(&kind) {
        return Err(format!("use {text}: kind is one of {}", KINDS.join(", ")));
    }
    let (set, icon) = match (kind, name) {
        ("icon", None) | ("icon", Some("*")) => ("*".to_string(), "*".to_string()),
        ("icon", Some(n)) => match n.split_once('/') {
            Some((s, i)) => (s.to_string(), i.to_string()),
            None => return Err(format!("use {text}: an icon's name is <set>/<icon>")),
        },
        (_, None) => (String::new(), String::new()),
        (_, Some(_)) => return Err(format!("use {text}: only icons take a name")),
    };
    Ok(Want {
        kind: kind.to_string(),
        product: product.to_string(),
        set,
        icon,
    })
}

fn fits(pattern: &str, value: &str) -> bool {
    pattern == "*" || pattern == value
}

pub fn resolve(
    text: &str,
    products: &[&str],
    icons: &[(String, String, String)],
    exists: &dyn Fn(&str, &str) -> bool,
) -> Result<Vec<Pick>, String> {
    let want = parse_use(text)?;
    if want.product != "*" && !products.contains(&want.product.as_str()) {
        return Err(format!(
            "use {text}: no product {}; known: {}",
            want.product,
            products.join(", ")
        ));
    }
    let mut out = Vec::new();
    for product in products.iter().filter(|p| fits(&want.product, p)) {
        if want.kind == "icon" {
            for (p, set, icon) in icons {
                if p == product && fits(&want.set, set) && fits(&want.icon, icon) {
                    out.push(Pick {
                        kind: "icon".into(),
                        product: product.to_string(),
                        name: Some(format!("{set}/{icon}")),
                    });
                }
            }
        } else if exists(&want.kind, product) {
            out.push(Pick {
                kind: want.kind.clone(),
                product: product.to_string(),
                name: None,
            });
        } else if want.product != "*" {
            return Err(format!(
                "use {text}: there's no {} master for {product}",
                want.kind
            ));
        }
    }
    if out.is_empty() {
        return Err(format!("use {text} matches nothing"));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PRODUCTS: &[&str] = &["sample", "other", "third", "fourth"];

    fn icons() -> Vec<(String, String, String)> {
        [
            ("sample", "shapes", "dot"),
            ("sample", "shapes", "dot-big"),
            ("sample", "lines", "node"),
            ("other", "cards", "card"),
        ]
        .iter()
        .map(|(a, b, c)| (a.to_string(), b.to_string(), c.to_string()))
        .collect()
    }

    fn no_wordmark_for_fourth(kind: &str, product: &str) -> bool {
        !(kind == "wordmark" && product == "fourth")
    }

    #[test]
    fn every_logo_in_listed_order() {
        let picks = resolve("logo:*", PRODUCTS, &icons(), &no_wordmark_for_fourth).unwrap();
        let names: Vec<&str> = picks.iter().map(|p| p.product.as_str()).collect();
        assert_eq!(names, PRODUCTS);
    }

    #[test]
    fn a_wildcard_skips_missing_masters_but_a_named_one_fails() {
        let all = resolve("wordmark:*", PRODUCTS, &icons(), &no_wordmark_for_fourth).unwrap();
        assert_eq!(all.len(), 3);
        assert!(
            resolve(
                "wordmark:fourth",
                PRODUCTS,
                &icons(),
                &no_wordmark_for_fourth
            )
            .is_err()
        );
    }

    #[test]
    fn icons_by_product_by_set_and_one_by_one() {
        let every = resolve("icon:sample:*", PRODUCTS, &icons(), &no_wordmark_for_fourth).unwrap();
        assert_eq!(every.len(), 3);
        let set = resolve(
            "icon:sample:shapes/*",
            PRODUCTS,
            &icons(),
            &no_wordmark_for_fourth,
        )
        .unwrap();
        assert_eq!(set.len(), 2);
        let one = resolve(
            "icon:other:cards/card",
            PRODUCTS,
            &icons(),
            &no_wordmark_for_fourth,
        )
        .unwrap();
        assert_eq!(one[0].name.as_deref(), Some("cards/card"));
        assert_eq!(
            resolve("icon:*", PRODUCTS, &icons(), &no_wordmark_for_fourth)
                .unwrap()
                .len(),
            4
        );
    }

    #[test]
    fn bad_uses_say_why() {
        assert!(parse_use("logo").is_err());
        assert!(parse_use("sticker:sample").is_err());
        assert!(parse_use("logo:sample:big").is_err());
        assert!(parse_use("icon:sample:drop").is_err());
        assert!(resolve("logo:orbit", PRODUCTS, &icons(), &no_wordmark_for_fourth).is_err());
    }

    #[test]
    fn shots_are_checked_against_the_standard() {
        let good: Map<String, Value> = serde_json::from_str(
            r#"{"line": "x", "assets": [{"use": "logo:*", "layout": "ring"}]}"#,
        )
        .unwrap();
        assert!(check_shot("logos", &good).is_ok());
        let stray: Map<String, Value> =
            serde_json::from_str(r#"{"assets": [{"use": "logo:*"}], "close": "palindrome"}"#)
                .unwrap();
        assert!(check_shot("logos", &stray).is_err());
        let empty: Map<String, Value> = serde_json::from_str(r#"{"line": "x"}"#).unwrap();
        assert!(check_shot("logos", &empty).is_err());
    }

    #[test]
    fn closures_read_like_hd_renderer() {
        assert_eq!(Closure::read(None).unwrap(), Closure::Open);
        assert_eq!(
            Closure::read(Some(&Value::from("palindrome"))).unwrap(),
            Closure::Palindrome
        );
        let fade: Value = serde_json::from_str(r#"{"crossfade": 0.5}"#).unwrap();
        assert_eq!(Closure::read(Some(&fade)).unwrap(), Closure::Crossfade(0.5));
        assert!(Closure::read(Some(&Value::from("loop"))).is_err());
    }
}
