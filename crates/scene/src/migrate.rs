use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use toml_edit::{DocumentMut, Item, Key, TableLike, Value};

use crate::Id;
use crate::code;
use crate::paths::{PathError, deepest_common, folder, join, lexical, slashed, up_to};
use crate::spans::KeyPath;
use crate::types::FileKind;

pub(crate) struct Problem {
    pub(crate) path: KeyPath,
    pub(crate) code: &'static str,
    pub(crate) message: String,
}

pub(crate) struct Migration {
    pub(crate) text: String,
    pub(crate) bodies: BTreeMap<usize, String>,
    pub(crate) problems: Vec<Problem>,
}

const LISTS: &[&str] = &[
    "object",
    "light",
    "emitter",
    "mover",
    "rig",
    "tiles",
    "particles",
];
const MAPS: &[&str] = &["mesh", "content", "text", "sound"];

pub(crate) fn entry_kinds(kind: FileKind) -> (&'static [&'static str], &'static [&'static str]) {
    match kind {
        FileKind::Scene | FileKind::Prefab => (LISTS, MAPS),
        FileKind::Materials => (&[], &["materials"]),
        FileKind::Proxies => (&["proxy"], &[]),
        FileKind::Project | FileKind::Finish => (&[], &[]),
    }
}

pub(crate) fn derived(file: &str, kind: &str, name: &str, taken: &BTreeSet<Id>) -> Id {
    let base = format!("{file}\n{kind}\n{name}");
    let mut id = Id::derive(base.as_bytes());
    let mut count = 0u32;
    while taken.contains(&id) {
        count += 1;
        id = Id::derive(format!("{base}\n{count}").as_bytes());
    }
    id
}

fn written_ids(doc: &DocumentMut, kind: FileKind) -> BTreeSet<Id> {
    let mut taken = BTreeSet::new();
    let (lists, maps) = entry_kinds(kind);
    for list in lists {
        for table in entries_of_list(doc.get(list)) {
            if let Some(id) = table
                .get("id")
                .and_then(Item::as_str)
                .and_then(|text| Id::parse(text).ok())
            {
                taken.insert(id);
            }
        }
    }
    for map in maps {
        if let Some(table) = doc.get(map).and_then(Item::as_table_like) {
            for (_, item) in table.iter() {
                if let Some(id) = item
                    .as_table_like()
                    .and_then(|entry| entry.get("id"))
                    .and_then(Item::as_str)
                    .and_then(|text| Id::parse(text).ok())
                {
                    taken.insert(id);
                }
            }
        }
    }
    taken
}

fn entries_of_list(item: Option<&Item>) -> Vec<&dyn TableLike> {
    match item {
        Some(Item::ArrayOfTables(array)) => {
            array.iter().map(|table| table as &dyn TableLike).collect()
        }
        Some(Item::Value(Value::Array(array))) => array
            .iter()
            .filter_map(|value| value.as_inline_table().map(|table| table as &dyn TableLike))
            .collect(),
        _ => Vec::new(),
    }
}

fn for_each_list_entry(item: Option<&mut Item>, mut each: impl FnMut(usize, &mut dyn TableLike)) {
    match item {
        Some(Item::ArrayOfTables(array)) => {
            for (place, table) in array.iter_mut().enumerate() {
                each(place, table);
            }
        }
        Some(Item::Value(Value::Array(array))) => {
            for (place, value) in array.iter_mut().enumerate() {
                if let Some(table) = value.as_inline_table_mut() {
                    each(place, table);
                }
            }
        }
        _ => {}
    }
}

pub(crate) fn add_ids(
    doc: &mut DocumentMut,
    file: &str,
    kind: FileKind,
    taken: &mut BTreeSet<Id>,
) -> Vec<(KeyPath, Id)> {
    taken.extend(written_ids(doc, kind));
    let mut added = Vec::new();
    let (lists, maps) = entry_kinds(kind);
    for list in lists {
        for_each_list_entry(doc.get_mut(list), |place, table| {
            if table.contains_key("id") {
                return;
            }
            let name = match table.get("name").and_then(Item::as_str) {
                Some(name) => name.to_string(),
                None if *list == "proxy" => {
                    let object = table
                        .get("object")
                        .and_then(Item::as_str)
                        .unwrap_or_default();
                    format!("{object}\n{place}")
                }
                None => return,
            };
            let id = derived(file, list, &name, taken);
            taken.insert(id);
            table.insert("id", toml_edit::value(id.to_string()));
            added.push((
                vec![list.to_string(), place.to_string(), "id".to_string()],
                id,
            ));
        });
    }
    for map in maps {
        let Some(table) = doc.get_mut(map).and_then(Item::as_table_like_mut) else {
            continue;
        };
        let names: Vec<String> = table.iter().map(|(key, _)| key.to_string()).collect();
        for name in names {
            let Some(entry) = table.get_mut(&name).and_then(Item::as_table_like_mut) else {
                continue;
            };
            if entry.contains_key("id") {
                continue;
            }
            let id = derived(file, map, &name, taken);
            taken.insert(id);
            entry.insert("id", toml_edit::value(id.to_string()));
            added.push((vec![map.to_string(), name, "id".to_string()], id));
        }
    }
    added
}

fn rename(table: &mut dyn TableLike, from: &str, to: &str) {
    if !table.contains_key(from) || table.contains_key(to) {
        return;
    }
    let keys: Vec<String> = table.iter().map(|(key, _)| key.to_string()).collect();
    let mut entries: Vec<(Key, Item)> = Vec::new();
    for key in keys {
        let found = table.get_key_value(&key).map(|(key, _)| key.clone());
        if let (Some(formatted), Some(item)) = (found, table.remove(&key)) {
            entries.push((formatted, item));
        }
    }
    for (key, item) in entries {
        let name = if key.get() == from { to } else { key.get() }.to_string();
        table.insert(&name, item);
        if let Some(mut slot) = table.key_mut(&name) {
            *slot.leaf_decor_mut() = key.leaf_decor().clone();
            *slot.dotted_decor_mut() = key.dotted_decor().clone();
        }
    }
}

struct PathValue {
    path: KeyPath,
}

fn path_keys(doc: &DocumentMut) -> Vec<PathValue> {
    let mut out = Vec::new();
    let mut push = |path: Vec<&str>| {
        out.push(PathValue {
            path: path.into_iter().map(str::to_string).collect(),
        })
    };
    for list in ["include", "materials"] {
        if let Some(array) = doc.get(list).and_then(Item::as_array) {
            for place in 0..array.len() {
                push(vec![list, &place.to_string()]);
            }
        }
    }
    for (map, key) in [
        ("mesh", "file"),
        ("content", "image"),
        ("text", "font"),
        ("sound", "file"),
    ] {
        if let Some(table) = doc.get(map).and_then(Item::as_table_like) {
            for (name, _) in table.iter() {
                push(vec![map, name, key]);
            }
        }
    }
    if doc.get("sky").is_some() {
        push(vec!["sky", "path"]);
        if let Some(layers) = doc.get("sky").and_then(|sky| sky.get("layer")) {
            let count = match layers {
                Item::ArrayOfTables(array) => array.len(),
                Item::Value(Value::Array(array)) => array.len(),
                _ => 0,
            };
            for place in 0..count {
                push(vec!["sky", "layer", &place.to_string(), "path"]);
            }
        }
    }
    push(vec!["sound"]);
    push(vec!["finish", "file"]);
    push(vec!["finish", "lut"]);
    if let Some(passes) = doc.get("finish").and_then(|finish| finish.get("pass")) {
        let count = match passes {
            Item::ArrayOfTables(array) => array.len(),
            Item::Value(Value::Array(array)) => array.len(),
            _ => 0,
        };
        for place in 0..count {
            push(vec!["finish", "pass", &place.to_string(), "lut"]);
        }
    }
    push(vec!["plates", "dir"]);
    push(vec!["plates", "proxies"]);
    out
}

fn in_item<'a>(item: &'a mut Item, path: &[String]) -> Option<&'a mut Value> {
    match item {
        Item::Value(value) => in_value(value, path),
        Item::Table(table) => in_table(table, path),
        Item::ArrayOfTables(array) => {
            let (first, rest) = path.split_first()?;
            in_table(array.get_mut(first.parse().ok()?)?, rest)
        }
        Item::None => None,
    }
}

fn in_table<'a>(table: &'a mut toml_edit::Table, path: &[String]) -> Option<&'a mut Value> {
    let (first, rest) = path.split_first()?;
    in_item(table.get_mut(first)?, rest)
}

fn in_value<'a>(value: &'a mut Value, path: &[String]) -> Option<&'a mut Value> {
    let Some((first, rest)) = path.split_first() else {
        return Some(value);
    };
    match value {
        Value::Array(array) => in_value(array.get_mut(first.parse().ok()?)?, rest),
        Value::InlineTable(table) => in_value(table.get_mut(first)?, rest),
        _ => None,
    }
}

fn value_at<'a>(doc: &'a mut DocumentMut, path: &[String]) -> Option<&'a mut Value> {
    in_table(doc.as_table_mut(), path)
}

fn set_text(value: &mut Value, text: &str) {
    let decor = value.decor().clone();
    let mut made = Value::from(text);
    *made.decor_mut() = decor;
    *value = made;
}

pub(crate) fn migrate(
    text: &str,
    file: &Path,
    root: &Path,
    kind: FileKind,
    strict: bool,
) -> Result<Migration, Vec<Problem>> {
    let mut doc: DocumentMut = text.parse().map_err(|error: toml_edit::TomlError| {
        vec![Problem {
            path: Vec::new(),
            code: code::SYNTAX,
            message: error.message().trim().to_string(),
        }]
    })?;
    let mut problems = Vec::new();
    let mut bodies = BTreeMap::new();
    doc.remove("format");
    let base = folder(file);
    if matches!(kind, FileKind::Scene | FileKind::Prefab) {
        let mut outside = Vec::new();
        let mut targets = vec![lexical(&root.join(&base))];
        for key in path_keys(&doc) {
            let Some(value) = value_at(&mut doc, &key.path) else {
                continue;
            };
            let Some(raw) = value.as_str().map(str::to_string) else {
                continue;
            };
            targets.push(
                lexical(&root.join(&base).join(&raw))
                    .parent()
                    .map(Path::to_path_buf)
                    .unwrap_or_default(),
            );
            match join(&base, &raw) {
                Ok(new) => set_text(value, &new),
                Err(PathError::Outside(shown)) => {
                    set_text(value, &shown);
                    outside.push((key.path, raw));
                }
                Err(_) => {}
            }
        }
        if !outside.is_empty() {
            let common: PathBuf = deepest_common(&targets);
            let at = up_to(&lexical(root), &common);
            for (path, raw) in outside {
                problems.push(Problem {
                    path,
                    code: code::OUTSIDE_ROOT,
                    message: format!(
                        "path {raw} leaves the project root; a project.toml in {at} (from the root) would hold this file and every file it names"
                    ),
                });
            }
        }
        for_each_list_entry(doc.get_mut("object"), |_, table| {
            rename(table, "id", "pick")
        });
        let names: Vec<Option<String>> = entries_of_list(doc.get("object"))
            .iter()
            .map(|table| table.get("name").and_then(Item::as_str).map(str::to_string))
            .collect();
        let mut moved: Vec<(usize, Item)> = Vec::new();
        if let Some(body) = doc.get_mut("body").and_then(Item::as_table_like_mut) {
            let keys: Vec<String> = body.iter().map(|(key, _)| key.to_string()).collect();
            for key in keys {
                match names
                    .iter()
                    .position(|name| name.as_deref() == Some(key.as_str()))
                {
                    Some(place) => {
                        if let Some(item) = body.remove(&key) {
                            moved.push((place, item));
                            bodies.insert(place, key);
                        }
                    }
                    None => {
                        problems.push(Problem {
                            path: vec!["body".to_string(), key.clone()],
                            code: code::MIGRATION,
                            message: format!(
                                "[body.{key}] names no object of this file; move it into its object as [object.body] by hand"
                            ),
                        });
                        if !strict {
                            body.remove(&key);
                        }
                    }
                }
            }
            if body.is_empty() {
                doc.remove("body");
            }
        }
        for (place, item) in moved {
            let item = match item {
                Item::Table(mut table) => {
                    table.set_position(None);
                    Item::Table(table)
                }
                Item::Value(Value::InlineTable(table)) => {
                    let mut table = table.into_table();
                    table.set_position(None);
                    Item::Table(table)
                }
                other => other,
            };
            match doc.get_mut("object") {
                Some(Item::ArrayOfTables(array)) => {
                    if let Some(table) = array.get_mut(place) {
                        table.insert("body", item);
                    }
                }
                Some(Item::Value(Value::Array(array))) => {
                    if let (Some(Value::InlineTable(table)), Some(value)) =
                        (array.get_mut(place), item.into_value().ok())
                    {
                        table.insert("body", value);
                    }
                }
                _ => {}
            }
        }
    }
    if strict && !problems.is_empty() {
        return Err(problems);
    }
    let mut taken = BTreeSet::new();
    add_ids(&mut doc, &slashed(file), kind, &mut taken);
    let rest = doc.to_string();
    let text = if rest.is_empty() {
        "format = 1\n".to_string()
    } else if rest.starts_with('\n') {
        format!("format = 1\n{rest}")
    } else {
        format!("format = 1\n\n{rest}")
    };
    Ok(Migration {
        text,
        bodies,
        problems,
    })
}
