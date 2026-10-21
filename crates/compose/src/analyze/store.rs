use std::path::Path;

use pfx_file::{Codec, RCPE, Reader, SCNE, Writer};
use serde::Serialize;

use crate::doc::{Compose, write_style};
use crate::style::Style;

pub const NEW_CHUNK: &str = "main.toml";

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Written {
    pub created: bool,
    pub replaced: bool,
    pub chunk: String,
}

fn parsed(text: &str) -> Option<toml::Table> {
    text.parse::<toml::Table>().ok()
}

fn has_compose(table: Option<&toml::Table>) -> bool {
    table.is_some_and(|table| table.contains_key("compose"))
}

fn has_style(table: Option<&toml::Table>, name: &str) -> bool {
    table.is_some_and(|table| {
        table
            .get("compose")
            .and_then(|c| c.get("style"))
            .and_then(|s| s.get(name))
            .is_some()
    })
}

fn users(table: Option<&toml::Table>, name: &str) -> Vec<String> {
    let Some(scores) = table
        .and_then(|t| t.get("compose"))
        .and_then(|c| c.get("score"))
        .and_then(toml::Value::as_table)
    else {
        return Vec::new();
    };
    scores
        .iter()
        .filter(|(_, score)| score.get("style").and_then(toml::Value::as_str) == Some(name))
        .map(|(score, _)| score.clone())
        .collect()
}

fn still_reads(text: &str, fresh: &str, name: &str, chunk: &str) -> Result<(), String> {
    if let Err(e) = Compose::parse(text) {
        return Err(format!(
            "`{chunk}` already fails to read, so analyze leaves it alone: {e}"
        ));
    }
    let Err(e) = Compose::parse(fresh) else {
        return Ok(());
    };
    let users = users(parsed(text).as_ref(), name);
    let because = if users.is_empty() {
        String::new()
    } else {
        format!(
            "{} composed from style {name} and would no longer read: ",
            users.join(", ")
        )
    };
    Err(format!(
        "`{chunk}`: writing [compose.style.{name}] here would leave a document no reader opens; {because}{e}; pick another --name"
    ))
}

fn create(path: &Path, name: &str, style: &Style) -> Result<Written, String> {
    let shown = path.display();
    let text = write_style("", name, style)?;
    Writer::new()
        .named(SCNE, NEW_CHUNK, text.into_bytes(), Codec::Auto)
        .write(path)
        .map_err(|e| format!("{shown}: {e}"))?;
    Ok(Written {
        created: true,
        replaced: false,
        chunk: NEW_CHUNK.to_string(),
    })
}

pub fn write(path: &Path, name: &str, style: &Style) -> Result<Written, String> {
    let shown = path.display();
    let meta = match std::fs::metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return create(path, name, style);
        }
        Err(e) => return Err(format!("{shown}: {e}")),
    };
    if meta.is_dir() || !pfx_file::has_magic(path) {
        return Err(format!(
            "{shown} exists and is not a .pitofx; --into wants a .pitofx or a new path"
        ));
    }
    let bytes = std::fs::read(path).map_err(|e| format!("{shown}: {e}"))?;
    let reader = Reader::from_bytes(&bytes).map_err(|e| format!("{shown}: {e}"))?;
    let mut first = None;
    let mut composed = None;
    let mut holder = None;
    let texts = [SCNE, RCPE].into_iter().flat_map(|tag| {
        reader
            .entries()
            .iter()
            .enumerate()
            .filter(move |(_, entry)| entry.tag == tag)
    });
    for (index, entry) in texts {
        let raw = reader.read(index).map_err(|e| format!("{shown}: {e}"))?;
        let text = String::from_utf8(raw.into_owned())
            .map_err(|_| format!("{shown}: `{}` is not UTF-8", entry.name))?;
        let table = parsed(&text);
        if holder.is_none() && has_style(table.as_ref(), name) {
            holder = Some((index, text.clone()));
        }
        if composed.is_none() && has_compose(table.as_ref()) {
            composed = Some((index, text.clone()));
        }
        if first.is_none() {
            first = Some((index, text));
        }
    }
    let replaced = holder.is_some();
    let target = holder.or(composed).or(first);
    let fresh = match &target {
        Some((index, text)) => {
            let chunk = &reader.entries()[*index].name;
            let fresh =
                write_style(text, name, style).map_err(|e| format!("{shown}: `{chunk}`: {e}"))?;
            still_reads(text, &fresh, name, chunk).map_err(|e| format!("{shown}: {e}"))?;
            fresh
        }
        None => write_style("", name, style)?,
    };
    let mut writer = Writer::new();
    let mut chunk = None;
    for (index, entry) in reader.entries().iter().enumerate() {
        if target.as_ref().is_some_and(|(at, _)| *at == index) {
            writer = writer.named(entry.tag, entry.name.clone(), fresh.as_bytes(), Codec::Auto);
            chunk = Some(entry.name.clone());
            continue;
        }
        writer = writer.keep(&reader, index);
    }
    let chunk = match chunk {
        Some(chunk) => chunk,
        None => {
            writer = writer.named(SCNE, NEW_CHUNK, fresh.as_bytes(), Codec::Auto);
            NEW_CHUNK.to_string()
        }
    };
    writer.write(path).map_err(|e| format!("{shown}: {e}"))?;
    Ok(Written {
        created: false,
        replaced,
        chunk,
    })
}
