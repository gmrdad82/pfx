use toml_edit::{Item, Table, Value};

use crate::include::Edit;
use crate::patch::{edit_root, morph_room};
use crate::read::{Main, read_main};
use crate::{Error, Key, Output, Recipe, SCENE_SUFFIX, Variant};

pub(crate) fn glob_matches(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    let (mut pi, mut ti) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some((pi, ti));
            pi += 1;
        } else if let Some((sp, st)) = star {
            pi = sp + 1;
            ti = st + 1;
            star = Some((sp, st + 1));
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

pub(crate) fn keys(dotted: &str) -> Result<Vec<Key>, String> {
    let mut out = Vec::new();
    let mut rest = dotted.trim();
    let mut first = true;
    while !rest.is_empty() {
        if !first {
            rest = match rest.strip_prefix('.') {
                Some(after) => after.trim_start(),
                None if rest.starts_with('[') => rest,
                None => return Err(format!("expected `.` or `[` before `{rest}`")),
            };
            if rest.is_empty() {
                return Err("ends with a `.`".to_string());
            }
        }
        first = false;
        if let Some(after) = rest.strip_prefix('"') {
            let mut name = String::new();
            let mut chars = after.char_indices();
            let end = loop {
                match chars.next() {
                    Some((i, '"')) => break i + 1,
                    Some((_, '\\')) => match chars.next() {
                        Some((_, c)) => name.push(c),
                        None => return Err("ends inside a quoted name".to_string()),
                    },
                    Some((_, c)) => name.push(c),
                    None => return Err("has an unclosed `\"`".to_string()),
                }
            };
            if name.is_empty() {
                return Err("has an empty quoted name".to_string());
            }
            out.push(Key::Table(name));
            rest = after[end..].trim_start();
        } else if let Some(after) = rest.strip_prefix('[') {
            let Some(close) = after.find(']') else {
                return Err("has an unclosed `[`".to_string());
            };
            let inner = after[..close].trim();
            if inner.is_empty() {
                return Err("has an empty `[]`".to_string());
            }
            out.push(if inner.bytes().all(|b| b.is_ascii_digit()) {
                Key::Index(
                    inner
                        .parse()
                        .map_err(|_| format!("`{inner}` is too large an index"))?,
                )
            } else {
                Key::Object(inner.to_string())
            });
            rest = after[close + 1..].trim_start();
        } else {
            let end = rest.find(['.', '[']).unwrap_or(rest.len());
            let name = rest[..end].trim();
            if name.is_empty() {
                return Err("has an empty key".to_string());
            }
            if name.contains(['"', ']']) {
                return Err(format!("`{name}` has a stray quote or bracket"));
            }
            out.push(Key::Table(name.to_string()));
            rest = &rest[end..];
        }
    }
    if out.is_empty() {
        return Err("is empty".to_string());
    }
    Ok(out)
}

fn toml_value(value: &serde_json::Value) -> Option<Value> {
    Some(match value {
        serde_json::Value::Null => return None,
        serde_json::Value::Bool(b) => Value::from(*b),
        serde_json::Value::Number(n) => match n.as_i64() {
            Some(i) => Value::from(i),
            None => Value::from(n.as_f64()?),
        },
        serde_json::Value::String(s) => Value::from(s.as_str()),
        serde_json::Value::Array(list) => {
            let mut array = toml_edit::Array::new();
            for element in list {
                array.push(toml_value(element)?);
            }
            Value::Array(array)
        }
        serde_json::Value::Object(map) => {
            let mut table = toml_edit::InlineTable::new();
            for (k, v) in map {
                table.insert(k.as_str(), toml_value(v)?);
            }
            Value::InlineTable(table)
        }
    })
}

fn suffixed(path: &str, name: &str) -> String {
    let file_at = path.rfind('/').map_or(0, |i| i + 1);
    match path[file_at..].rfind('.') {
        Some(dot) if dot > 0 => {
            let cut = file_at + dot;
            format!("{}-{name}{}", &path[..cut], &path[cut..])
        }
        _ => format!("{path}-{name}"),
    }
}

fn scene_suffixed(path: &str, name: &str) -> String {
    match path.strip_suffix(SCENE_SUFFIX) {
        Some(stem) if !stem.is_empty() && !stem.ends_with('/') => {
            format!("{stem}-{name}{SCENE_SUFFIX}")
        }
        _ => suffixed(path, name),
    }
}

fn variant_outputs(recipe: &str, name: &str, base: &Output, variant: &Variant) -> Output {
    let own = &variant.output;
    Output {
        glb: Some(own.glb.clone().unwrap_or_else(|| {
            suffixed(
                base.glb.as_deref().unwrap_or(&format!("{recipe}.glb")),
                name,
            )
        })),
        sidecar: own
            .sidecar
            .clone()
            .or_else(|| base.sidecar.as_deref().map(|p| suffixed(p, name))),
        bake: own.bake.clone(),
        scene: own
            .scene
            .clone()
            .or_else(|| base.scene.as_deref().map(|p| scene_suffixed(p, name))),
        ..base.clone()
    }
}

fn set_outputs(doc: &mut Table, out: &Output, base: &Output) -> Result<(), String> {
    let item = doc
        .entry("output")
        .or_insert_with(|| Item::Table(Table::new()));
    let table = item
        .as_table_like_mut()
        .ok_or_else(|| "`output` is not a table".to_string())?;
    if let Some(glb) = &out.glb {
        table.insert("glb", Item::Value(Value::from(glb.as_str())));
    }
    if let Some(sidecar) = &out.sidecar {
        table.insert("sidecar", Item::Value(Value::from(sidecar.as_str())));
    }
    if let Some(scene) = &out.scene {
        table.insert("scene", Item::Value(Value::from(scene.as_str())));
    }
    let drop_bake = out.bake.is_none() && base.bake.is_some();
    match &out.bake {
        Some(bake) => {
            table.insert("bake", Item::Value(Value::from(bake.as_str())));
        }
        None => {
            table.remove("bake");
        }
    }
    if drop_bake {
        doc.remove("bake");
    }
    Ok(())
}

pub(crate) fn apply_to_table(
    doc: &mut Table,
    name: &str,
    recipe: &str,
    base: &Output,
    variant: &Variant,
) -> Result<(), Error> {
    let at = format!("variant.{name}");
    let out = variant_outputs(recipe, name, base, variant);
    set_outputs(doc, &out, base).map_err(|text| Error::new(format!("{at}.output"), None, text))?;
    if !variant.drop.is_empty()
        && let Some(list) = doc.get_mut("object").and_then(Item::as_array_of_tables_mut)
    {
        list.retain(|t| {
            let label = t.get("name").and_then(Item::as_str).unwrap_or("");
            !variant.drop.iter().any(|g| glob_matches(g, label))
        });
    }
    for (dotted, value) in &variant.set {
        let value = toml_value(value).ok_or_else(|| {
            Error::new(
                format!("{at}.set.{dotted}"),
                None,
                "expected a number, string, boolean, array or table",
            )
        })?;
        let path =
            keys(dotted).map_err(|why| Error::new(format!("{at}.set.{dotted}"), None, why))?;
        morph_room(doc, &path);
        edit_root(doc, &path, Some(Item::Value(value)))
            .map_err(|text| Error::new(format!("{at}.set.{dotted}"), None, text))?;
    }
    for dotted in &variant.remove {
        let path = keys(dotted)
            .map_err(|why| Error::new(format!("{at}.remove"), None, format!("`{dotted}` {why}")))?;
        edit_root(doc, &path, None).map_err(|text| {
            Error::new(format!("{at}.remove"), None, format!("`{dotted}`: {text}"))
        })?;
    }
    doc.remove("variant");
    Ok(())
}

pub(crate) fn replay(r: &Recipe, edit: Edit) -> Result<Recipe, Error> {
    let mut edits = r.files.edits.clone();
    edits.push(edit);
    let memory = r.files.memory();
    read_main(Main {
        text: r.files.main_text().unwrap_or(&r.source),
        base: r.base.as_deref(),
        name: r.files.main_name(),
        memory: &memory,
        edits: &edits,
    })
}

pub fn apply_variant(r: &Recipe, name: &str) -> Result<Recipe, Error> {
    let Some(variant) = r.variants.get(name) else {
        let known: Vec<&str> = r.variants.keys().map(String::as_str).collect();
        let hint = if known.is_empty() {
            "the recipe has no variants".to_string()
        } else {
            format!("expected one of: {}", known.join(", "))
        };
        return Err(Error::new(
            format!("variant.{name}"),
            None,
            format!("no such variant, {hint}"),
        ));
    };
    replay(
        r,
        Edit::Variant {
            name: name.to_string(),
            recipe: r.name.clone(),
            output: Box::new(r.output.clone()),
            variant: Box::new(variant.clone()),
        },
    )
    .map_err(|mut e| {
        if e.line.is_none() {
            e.line = r.line_of(&format!("variant.{name}"));
        }
        e
    })
}

#[cfg(test)]
mod tests {
    use super::{glob_matches, scene_suffixed, suffixed};

    #[test]
    fn globs_match_stars_and_question_marks() {
        assert!(glob_matches("slat*", "slat"));
        assert!(glob_matches("slat*", "slat_12"));
        assert!(glob_matches("*_knob", "mv_knob"));
        assert!(glob_matches("a*b*c", "aXXbYYc"));
        assert!(glob_matches("wall_?", "wall_3"));
        assert!(glob_matches("*", ""));
        assert!(glob_matches("exact", "exact"));
        assert!(!glob_matches("slat*", "wall"));
        assert!(!glob_matches("wall_?", "wall_12"));
        assert!(!glob_matches("a*b", "acd"));
        assert!(!glob_matches("exact", "exactly"));
        assert!(!glob_matches("", "x"));
    }

    #[test]
    fn a_variant_name_goes_before_the_last_extension_of_the_file_name() {
        assert_eq!(suffixed("deck.glb", "bare"), "deck-bare.glb");
        assert_eq!(suffixed("a/b.c/deck.json", "bare"), "a/b.c/deck-bare.json");
        assert_eq!(suffixed("a/deck.tar.gz", "x"), "a/deck.tar-x.gz");
        assert_eq!(suffixed("a.b/deck", "x"), "a.b/deck-x");
        assert_eq!(suffixed(".hidden", "x"), ".hidden-x");
    }

    #[test]
    fn a_variant_name_goes_before_the_scene_suffix_of_a_scene_file() {
        assert_eq!(
            scene_suffixed("out/deck.scene.toml", "bare"),
            "out/deck-bare.scene.toml"
        );
        assert_eq!(scene_suffixed("deck.toml", "bare"), "deck-bare.toml");
        assert_eq!(scene_suffixed("a/.scene.toml", "x"), "a/.scene-x.toml");
    }
}
