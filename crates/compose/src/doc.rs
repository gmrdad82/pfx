use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use toml_edit::{Array, DocumentMut, Item, Table, Value};

use crate::score::{Score, StyleRef};
use crate::style::Style;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Compose {
    pub styles: BTreeMap<String, Style>,
    pub scores: BTreeMap<String, Score>,
}

#[derive(Deserialize)]
struct Doc {
    compose: Option<Raw>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    #[serde(default)]
    style: BTreeMap<String, Style>,
    #[serde(default)]
    score: BTreeMap<String, Score>,
}

fn header(line: &str) -> Option<&str> {
    let line = line.split('#').next().unwrap_or(line).trim();
    let inner = line
        .strip_prefix("[[")
        .and_then(|l| l.strip_suffix("]]"))
        .or_else(|| line.strip_prefix('[').and_then(|l| l.strip_suffix(']')))?
        .trim();
    let named = !inner.is_empty()
        && inner.parse::<f64>().is_err()
        && inner
            .chars()
            .all(|c| c.is_alphanumeric() || "_-.\"' ".contains(c));
    named.then_some(line)
}

fn header_before(text: &str, at: usize) -> Option<&str> {
    let head = &text[..at.min(text.len())];
    head.lines().rev().find_map(header)
}

fn explain(text: &str, error: &toml::de::Error) -> String {
    let message = error.message().trim();
    match error.span() {
        Some(span) => {
            let line = text[..span.start.min(text.len())].lines().count().max(1);
            match header_before(text, span.start) {
                Some(header) => format!("{header}: {message} (line {line})"),
                None => format!("{message} (line {line})"),
            }
        }
        None => message.to_string(),
    }
}

impl Compose {
    pub fn parse(text: &str) -> Result<Compose, String> {
        let doc: Doc = toml::from_str(text).map_err(|e| explain(text, &e))?;
        let Some(raw) = doc.compose else {
            return Ok(Compose::default());
        };
        for (name, style) in &raw.style {
            style
                .check()
                .map_err(|e| format!("[compose.style.{name}] {e}"))?;
        }
        for (name, score) in &raw.score {
            let at = format!("[compose.score.{name}]");
            score.check().map_err(|e| format!("{at} {e}"))?;
            if let StyleRef::Name(style) = &score.style {
                let style = raw.style.get(style).ok_or_else(|| {
                    format!("{at} `style = \"{style}\"` names no [compose.style.{style}] in the document")
                })?;
                score.check_style(style).map_err(|e| format!("{at} {e}"))?;
            }
        }
        Ok(Compose {
            styles: raw.style,
            scores: raw.score,
        })
    }

    pub fn is_empty(&self) -> bool {
        self.styles.is_empty() && self.scores.is_empty()
    }

    pub fn style(&self, name: &str) -> Result<&Style, String> {
        self.styles.get(name).ok_or_else(|| {
            let known: Vec<&str> = self.styles.keys().map(String::as_str).collect();
            if known.is_empty() {
                format!("no [compose.style.{name}]; the document has no styles")
            } else {
                format!(
                    "no [compose.style.{name}]; the document has {}",
                    known.join(", ")
                )
            }
        })
    }

    pub fn score(&self, name: &str) -> Result<&Score, String> {
        self.scores
            .get(name)
            .ok_or_else(|| format!("no [compose.score.{name}]"))
    }
}

#[derive(Clone, Debug)]
pub struct Found {
    pub chunk: String,
    pub text: String,
    pub compose: Compose,
}

pub fn find(reader: &pfx_file::Reader) -> Result<Vec<Found>, String> {
    let mut out = Vec::new();
    for (chunk, bytes) in reader.chunks(pfx_file::SCNE).map_err(|e| e.to_string())? {
        let text =
            String::from_utf8(bytes.into_owned()).map_err(|_| format!("`{chunk}` is not UTF-8"))?;
        let compose = Compose::parse(&text).map_err(|e| format!("`{chunk}`: {e}"))?;
        if !compose.is_empty() {
            out.push(Found {
                chunk: chunk.to_string(),
                text,
                compose,
            });
        }
    }
    Ok(out)
}

fn place_tables(item: &mut Item, at: isize) {
    match item {
        Item::Table(table) => {
            table.set_position(Some(at));
            for (_, child) in table.iter_mut() {
                place_tables(child, at);
            }
        }
        Item::ArrayOfTables(array) => {
            for table in array.iter_mut() {
                table.set_position(Some(at));
                for (_, child) in table.iter_mut() {
                    place_tables(child, at);
                }
            }
        }
        _ => {}
    }
}

fn first_position(item: &Item) -> Option<isize> {
    match item {
        Item::Table(table) => {
            let own = table.position().filter(|_| !table.is_implicit());
            let children = table.iter().filter_map(|(_, child)| first_position(child));
            own.into_iter().chain(children).min()
        }
        Item::ArrayOfTables(array) => array.iter().filter_map(|t| t.position()).min(),
        _ => None,
    }
}

fn last_position(item: &Item) -> Option<isize> {
    match item {
        Item::Table(table) => {
            let children = table.iter().filter_map(|(_, child)| last_position(child));
            table.position().into_iter().chain(children).max()
        }
        Item::ArrayOfTables(array) => array.iter().filter_map(|t| t.position()).max(),
        _ => None,
    }
}

fn implicit() -> Item {
    let mut table = Table::new();
    table.set_implicit(true);
    Item::Table(table)
}

fn shift_after(item: &mut Item, from: isize) {
    match item {
        Item::Table(table) => {
            if let Some(p) = table.position()
                && p >= from
            {
                table.set_position(Some(p + 1));
            }
            for (_, child) in table.iter_mut() {
                shift_after(child, from);
            }
        }
        Item::ArrayOfTables(array) => {
            for table in array.iter_mut() {
                if let Some(p) = table.position()
                    && p >= from
                {
                    table.set_position(Some(p + 1));
                }
                for (_, child) in table.iter_mut() {
                    shift_after(child, from);
                }
            }
        }
        _ => {}
    }
}

pub(crate) fn write_section<T: Serialize>(
    text: &str,
    kind: &str,
    name: &str,
    value: &T,
) -> Result<String, String> {
    let mut doc: DocumentMut = text
        .parse()
        .map_err(|e: toml_edit::TomlError| e.to_string())?;
    let body = toml::to_string(value).map_err(|e| e.to_string())?;
    let piece: DocumentMut = body
        .parse()
        .map_err(|e: toml_edit::TomlError| e.to_string())?;
    let mut fresh = Item::Table(piece.as_table().clone());
    rows(&mut fresh);
    let root = doc.as_item();
    let replaced = root
        .get("compose")
        .and_then(|c| c.get(kind))
        .and_then(|k| k.get(name))
        .and_then(first_position);
    let at = match replaced {
        Some(at) => at,
        None => {
            let compose = root.get("compose");
            let after = compose
                .and_then(last_position)
                .or_else(|| last_position(root));
            let at = after.map_or(0, |p| p + 1);
            shift_after(doc.as_item_mut(), at);
            at
        }
    };
    place_tables(&mut fresh, at);
    let compose = doc
        .as_table_mut()
        .entry("compose")
        .or_insert_with(implicit)
        .as_table_mut()
        .ok_or("`compose` is not a table")?;
    let kinds = compose
        .entry(kind)
        .or_insert_with(implicit)
        .as_table_mut()
        .ok_or_else(|| format!("`compose.{kind}` is not a table"))?;
    kinds.insert(name, fresh);
    Ok(doc.to_string())
}

pub fn write_score(text: &str, name: &str, score: &Score) -> Result<String, String> {
    score
        .check()
        .map_err(|e| format!("[compose.score.{name}] {e}"))?;
    write_section(text, "score", name, score)
}

pub fn write_style(text: &str, name: &str, style: &Style) -> Result<String, String> {
    style
        .check()
        .map_err(|e| format!("[compose.style.{name}] {e}"))?;
    write_section(text, "style", name, style)
}

fn one_per_line(array: &mut Array) {
    if array.is_empty() || !array.iter().all(|v| v.is_array()) {
        return;
    }
    for value in array.iter_mut() {
        value.decor_mut().set_prefix("\n  ");
        value.decor_mut().set_suffix("");
    }
    array.set_trailing("\n");
    array.set_trailing_comma(true);
}

fn rows(item: &mut Item) {
    match item {
        Item::Table(table) => {
            for (_, child) in table.iter_mut() {
                rows(child);
            }
        }
        Item::ArrayOfTables(array) => {
            for table in array.iter_mut() {
                for (_, child) in table.iter_mut() {
                    rows(child);
                }
            }
        }
        Item::Value(Value::Array(array)) => one_per_line(array),
        _ => {}
    }
}
