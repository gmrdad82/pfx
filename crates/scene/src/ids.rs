use std::collections::BTreeSet;

use toml_edit::{DocumentMut, Item, TableLike, Value};

use crate::Id;
use crate::spans::KeyPath;
use crate::types::FileKind;

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
