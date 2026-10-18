use std::collections::BTreeSet;
use std::sync::Mutex;

use pfx_recipe::toml_edit::{ArrayOfTables, DocumentMut, Item, Table, Value};
use pfx_recipe::{Key, Loaded, Patch, referrers};

pub const HEADER: &str = "# pfx objects 1";
const LINKS: [&str; 3] = ["parent", "on", "into"];

#[derive(Default)]
pub struct Board {
    text: Mutex<Option<String>>,
}

impl pfx_play::Clipboard for Board {
    fn set_text(&self, text: &str) -> bool {
        match self.text.lock() {
            Ok(mut held) => {
                *held = Some(text.to_string());
                true
            }
            Err(_) => false,
        }
    }

    fn get_text(&self) -> Option<String> {
        self.text.lock().ok()?.clone()
    }
}

#[derive(Clone, Debug)]
pub struct Pasted {
    pub patches: Vec<Patch>,
    pub names: Vec<String>,
    pub kept: Vec<String>,
}

fn objects(doc: &DocumentMut) -> Option<&ArrayOfTables> {
    doc.get("object").and_then(Item::as_array_of_tables)
}

fn name_of(table: &Table) -> Option<&str> {
    table.get("name").and_then(Item::as_str)
}

fn subtree(list: &ArrayOfTables, roots: &[String]) -> Vec<String> {
    let mut chosen: BTreeSet<String> = roots.iter().cloned().collect();
    loop {
        let before = chosen.len();
        for table in list.iter() {
            if let (Some(name), Some(parent)) =
                (name_of(table), table.get("parent").and_then(Item::as_str))
                && chosen.contains(parent)
            {
                chosen.insert(name.to_string());
            }
        }
        if chosen.len() == before {
            break;
        }
    }
    list.iter()
        .filter_map(name_of)
        .filter(|name| chosen.contains(*name))
        .map(str::to_string)
        .collect()
}

fn materials_of(table: &Table, into: &mut BTreeSet<String>) {
    if let Some(name) = table.get("material").and_then(Item::as_str) {
        into.insert(name.to_string());
    }
    match table.get("materials") {
        Some(Item::Value(Value::Array(array))) => {
            into.extend(array.iter().filter_map(Value::as_str).map(str::to_string));
        }
        Some(Item::Value(Value::InlineTable(inline))) => {
            into.extend(
                inline
                    .iter()
                    .filter_map(|(_, v)| v.as_str())
                    .map(str::to_string),
            );
        }
        Some(Item::Table(table)) => {
            into.extend(
                table
                    .iter()
                    .filter_map(|(_, v)| v.as_str())
                    .map(str::to_string),
            );
        }
        _ => {}
    }
}

pub fn copy(l: &Loaded, roots: &[String]) -> Result<String, String> {
    let list = objects(&l.doc).ok_or("the recipe has no objects")?;
    let names = subtree(list, roots);
    if names.is_empty() {
        return Err("nothing selected to copy".to_string());
    }
    let mut out = DocumentMut::new();
    let mut copied = ArrayOfTables::new();
    let mut materials = BTreeSet::new();
    for table in list.iter() {
        if name_of(table).is_some_and(|name| names.iter().any(|n| n == name)) {
            let mut table = table.clone();
            table.decor_mut().clear();
            materials_of(&table, &mut materials);
            copied.push(table);
        }
    }
    out.insert("object", Item::ArrayOfTables(copied));
    if let Some(library) = l.doc.get("materials").and_then(Item::as_table) {
        let mut kept = Table::new();
        for name in &materials {
            if let Some(item) = library.get(name) {
                let mut item = item.clone();
                if let Some(table) = item.as_table_mut() {
                    table.decor_mut().clear();
                }
                kept.insert(name, item);
            }
        }
        if !kept.is_empty() {
            kept.set_implicit(true);
            out.insert("materials", Item::Table(kept));
        }
    }
    Ok(format!("{HEADER}\n{out}"))
}

fn unique(taken: &BTreeSet<String>, name: &str) -> String {
    if !taken.contains(name) {
        return name.to_string();
    }
    let stem = match name.rsplit_once('_') {
        Some((stem, n)) if !stem.is_empty() && n.parse::<u32>().is_ok() => stem,
        _ => name,
    };
    (2..)
        .map(|n| format!("{stem}_{n}"))
        .find(|candidate| !taken.contains(candidate))
        .unwrap_or_else(|| name.to_string())
}

pub fn paste(l: &Loaded, text: &str) -> Result<Pasted, String> {
    let body = text
        .trim_start()
        .strip_prefix(HEADER)
        .ok_or("the clipboard holds no pfx objects")?;
    let clip: DocumentMut = body
        .parse()
        .map_err(|error| format!("the clipboard's objects do not read: {error}"))?;
    let list = objects(&clip).ok_or("the clipboard holds no objects")?;
    let mut taken: BTreeSet<String> = l.recipe.objects.iter().map(|o| o.name.clone()).collect();
    let mut renamed: Vec<(String, String)> = Vec::new();
    for table in list.iter() {
        let name = name_of(table).ok_or("a copied object has no name")?;
        let new = unique(&taken, name);
        taken.insert(new.clone());
        renamed.push((name.to_string(), new));
    }
    let mapped = |old: &str| {
        renamed
            .iter()
            .find(|(from, _)| from == old)
            .map(|(_, to)| to.clone())
    };
    let existing: BTreeSet<&str> = l.recipe.objects.iter().map(|o| o.name.as_str()).collect();
    let mut entries = Vec::new();
    for (table, (_, new)) in list.iter().zip(&renamed) {
        let mut table = table.clone();
        table.insert("name", Item::Value(Value::from(new.as_str())));
        for link in LINKS {
            let Some(target) = table.get(link).and_then(Item::as_str).map(str::to_string) else {
                continue;
            };
            match mapped(&target) {
                Some(to) => {
                    table.insert(link, Item::Value(Value::from(to)));
                }
                None if existing.contains(target.as_str()) => {}
                None => {
                    table.remove(link);
                }
            }
        }
        entries.push(table);
    }
    let mut patches = Vec::new();
    match objects(&l.doc) {
        Some(list) => {
            for (at, table) in entries.into_iter().enumerate() {
                patches.push(Patch {
                    path: vec![Key::Table("object".into()), Key::Index(list.len() + at)],
                    value: Some(Item::Table(table)),
                });
            }
        }
        None => {
            let mut list = ArrayOfTables::new();
            for table in entries {
                list.push(table);
            }
            patches.push(Patch {
                path: vec![Key::Table("object".into())],
                value: Some(Item::ArrayOfTables(list)),
            });
        }
    }
    let mut kept = Vec::new();
    if let Some(library) = clip.get("materials").and_then(Item::as_table) {
        let own = l.doc.get("materials").and_then(Item::as_table);
        let mut fresh = Table::new();
        for (name, item) in library.iter() {
            if own.is_some_and(|own| own.contains_key(name)) {
                kept.push(name.to_string());
            } else {
                fresh.insert(name, item.clone());
            }
        }
        match own {
            Some(_) => {
                for (name, item) in fresh.iter() {
                    patches.push(Patch {
                        path: vec![Key::Table("materials".into()), Key::Table(name.to_string())],
                        value: Some(item.clone()),
                    });
                }
            }
            None if !fresh.is_empty() => {
                fresh.set_implicit(true);
                patches.push(Patch {
                    path: vec![Key::Table("materials".into())],
                    value: Some(Item::Table(fresh)),
                });
            }
            None => {}
        }
    }
    Ok(Pasted {
        patches,
        names: renamed.into_iter().map(|(_, new)| new).collect(),
        kept,
    })
}

pub fn remove(l: &Loaded, roots: &[String]) -> Result<(Vec<Patch>, Vec<String>), String> {
    let list = objects(&l.doc).ok_or("the recipe has no objects")?;
    let names = subtree(list, roots);
    if names.is_empty() {
        return Err("nothing selected to remove".to_string());
    }
    for name in &names {
        let outside: Vec<String> = referrers(&l.recipe, "object", name)
            .into_iter()
            .filter(|by| {
                !names
                    .iter()
                    .any(|n| by.starts_with(&format!("object[{n}]")))
            })
            .collect();
        if let Some(by) = outside.first() {
            return Err(format!(
                "{by} refers to {name}; change or remove {by} first"
            ));
        }
    }
    let patches = names
        .iter()
        .rev()
        .map(|name| Patch {
            path: vec![Key::Table("object".into()), Key::Object(name.clone())],
            value: None,
        })
        .collect();
    Ok((patches, names))
}

#[cfg(test)]
mod tests {
    use super::*;

    const DESK: &str = r#"recipe = 1
name = "desk"

[timeline]
fps = 30
seconds = 2.0

[[object]]
name = "table"
kind = "box"
size = [1.0, 0.1, 1.0]
material = "oak"
keys = { at = [[0.0, [0.0, 0.0, 0.0]], [1.0, [0.0, 0.5, 0.0]]] }

[[object]]
name = "cup"
kind = "box"
size = [0.1, 0.1, 0.1]
parent = "table"
material = "enamel"

[[object]]
name = "lamp"
kind = "box"
size = [0.1, 0.3, 0.1]

[materials.oak]
family = "lacquer"
base = [0.5, 0.3, 0.1]

[materials.enamel]
family = "lacquer"
base = [0.9, 0.9, 0.9]
"#;

    fn loaded(name: &str, text: &str) -> Loaded {
        let path = crate::scratch(name).with_file_name(format!("{name}.toml"));
        std::fs::write(&path, text).unwrap();
        pfx_recipe::load(&path).unwrap()
    }

    fn applied(l: &Loaded, patches: &[Patch]) -> DocumentMut {
        let mut doc = l.doc.clone();
        for patch in patches {
            pfx_recipe::edit(&mut doc, patch).unwrap();
        }
        doc
    }

    #[test]
    fn a_copy_carries_the_subtree_its_materials_and_its_keys() {
        let l = loaded("clip-copy", DESK);
        let text = copy(&l, &["table".to_string()]).unwrap();
        assert!(text.starts_with(HEADER));
        let clip: DocumentMut = text[HEADER.len()..].parse().unwrap();
        let names: Vec<&str> = objects(&clip).unwrap().iter().filter_map(name_of).collect();
        assert_eq!(names, ["table", "cup"]);
        assert!(text.contains("keys = { at = "), "{text}");
        let mut materials: Vec<&str> = clip["materials"]
            .as_table()
            .unwrap()
            .iter()
            .map(|(k, _)| k)
            .collect();
        materials.sort();
        assert_eq!(materials, ["enamel", "oak"]);
    }

    #[test]
    fn a_paste_renames_relinks_and_adds_only_missing_materials() {
        let l = loaded("clip-paste", DESK);
        let text = copy(&l, &["table".to_string()]).unwrap();
        let pasted = paste(&l, &text).unwrap();
        assert_eq!(pasted.names, ["table_2", "cup_2"]);
        let mut kept = pasted.kept.clone();
        kept.sort();
        assert_eq!(kept, ["enamel", "oak"]);
        let doc = applied(&l, &pasted.patches);
        let list = objects(&doc).unwrap();
        let cup = list.iter().find(|t| name_of(t) == Some("cup_2")).unwrap();
        assert_eq!(cup.get("parent").and_then(Item::as_str), Some("table_2"));
        let other = loaded("clip-other", "recipe = 1\nname = \"other\"\n");
        let into = paste(&other, &text).unwrap();
        assert_eq!(into.names, ["table", "cup"]);
        let doc = applied(&other, &into.patches);
        assert_eq!(objects(&doc).unwrap().len(), 2);
        assert!(doc["materials"]["oak"].is_table());
        assert!(paste(&l, "name = \"x\"").is_err());
    }

    #[test]
    fn a_removal_takes_the_subtree_and_refuses_an_outside_reference() {
        let l = loaded("clip-remove", DESK);
        let (patches, names) = remove(&l, &["table".to_string()]).unwrap();
        assert_eq!(names, ["table", "cup"]);
        let doc = applied(&l, &patches);
        let left: Vec<&str> = objects(&doc).unwrap().iter().filter_map(name_of).collect();
        assert_eq!(left, ["lamp"]);
        let carried = loaded(
            "clip-carried",
            &DESK.replace("name = \"lamp\"\n", "name = \"lamp\"\nparent = \"cup\"\n"),
        );
        assert_eq!(
            remove(&carried, &["cup".to_string()]).unwrap().1,
            ["cup", "lamp"]
        );
        let pinned = loaded(
            "clip-pinned",
            &format!(
                "{DESK}\n[[object]]\nname = \"label\"\nkind = \"text\"\non = \"cup\"\nspans = [{{ text = \"x\", size = 0.02 }}]\n"
            ),
        );
        let refused = remove(&pinned, &["cup".to_string()]).unwrap_err();
        assert!(refused.contains("object[label].on"), "{refused}");
    }
}
