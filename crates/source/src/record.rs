use std::collections::BTreeMap;
use std::path::Path;

use pfx_file::{Codec, Reader, Writer};
use toml::{Table, Value};

use crate::{Asset, Kind};

pub const RECORD: &str = "asset.pitofx";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    pub asset: Asset,
    pub files: BTreeMap<String, String>,
    pub archive: Option<(String, String)>,
    pub pfx: String,
}

fn text(value: &str) -> Value {
    Value::String(value.to_string())
}

impl Record {
    pub fn facts(&self) -> String {
        let a = &self.asset;
        let mut root = Table::new();
        root.insert("licence".into(), text(&a.licence));
        root.insert("credit".into(), text(&a.credit));
        if let Some(author) = &a.author {
            root.insert("author".into(), text(author));
        }
        if let Some(url) = &a.url {
            root.insert("source".into(), text(url));
        }
        let mut asset = Table::new();
        asset.insert("from".into(), text(&a.source));
        asset.insert("id".into(), text(&a.id));
        asset.insert("name".into(), text(&a.name));
        asset.insert("kind".into(), text(a.kind.word()));
        asset.insert("pfx".into(), text(&self.pfx));
        if let Some((name, sha)) = &self.archive {
            asset.insert("archive".into(), text(name));
            asset.insert("archive_sha256".into(), text(sha));
        }
        root.insert("asset".into(), Value::Table(asset));
        root.insert(
            "files".into(),
            Value::Table(
                self.files
                    .iter()
                    .map(|(k, v)| (k.clone(), text(v)))
                    .collect(),
            ),
        );
        toml::to_string(&root).unwrap_or_default()
    }

    pub fn parse(facts: &str) -> Result<Record, String> {
        let root: Table = toml::from_str(facts).map_err(|e| e.to_string())?;
        let field =
            |table: &Table, key: &str| table.get(key).and_then(Value::as_str).map(str::to_string);
        let asset = root
            .get("asset")
            .and_then(Value::as_table)
            .ok_or("[asset]: missing, this .pitofx is not a fetched asset's record")?;
        let need = |key: &str| field(asset, key).ok_or_else(|| format!("asset.{key}: missing"));
        let files = root
            .get("files")
            .and_then(Value::as_table)
            .map(|t| {
                t.iter()
                    .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string())))
                    .collect()
            })
            .unwrap_or_default();
        let archive = field(asset, "archive").zip(field(asset, "archive_sha256"));
        Ok(Record {
            asset: Asset {
                source: need("from")?,
                id: need("id")?,
                name: need("name")?,
                kind: Kind::parse(&need("kind")?)?,
                author: field(&root, "author"),
                licence: field(&root, "licence").unwrap_or_default(),
                credit: field(&root, "credit").unwrap_or_default(),
                url: field(&root, "source"),
            },
            files,
            archive,
            pfx: need("pfx")?,
        })
    }

    pub fn write(&self, dir: &Path) -> Result<(), String> {
        let path = dir.join(RECORD);
        Writer::new()
            .chunk(pfx_file::FACT, self.facts().into_bytes(), Codec::Auto)
            .write(&path)
            .map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn read(dir: &Path) -> Result<Record, String> {
        let path = dir.join(RECORD);
        let reader = Reader::open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        Record::from_reader(&reader).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn from_reader(reader: &Reader<'_>) -> Result<Record, String> {
        let bytes = reader
            .chunk(pfx_file::FACT)
            .map_err(|e| e.to_string())?
            .ok_or("no FACT chunk")?;
        Record::parse(std::str::from_utf8(&bytes).map_err(|_| "the facts are not UTF-8")?)
    }

    pub fn json(&self) -> serde_json::Value {
        let a = &self.asset;
        serde_json::json!({
            "ref": format!("{}:{}", a.source, a.id),
            "source": a.source,
            "id": a.id,
            "name": a.name,
            "kind": a.kind.word(),
            "author": a.author,
            "licence": a.licence,
            "credit": a.credit,
            "url": a.url,
            "files": self.files,
            "archive": self.archive.as_ref().map(|(name, sha)| serde_json::json!({"name": name, "sha256": sha})),
            "pfx": self.pfx,
        })
    }
}

pub fn list(root: &Path) -> Result<Vec<(std::path::PathBuf, Record)>, String> {
    let mut out = Vec::new();
    if !root.is_dir() {
        return Ok(out);
    }
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        let entries = std::fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if path.join(RECORD).is_file() {
                    let record = Record::read(&path)?;
                    out.push((path, record));
                } else if depth < 2 {
                    stack.push((path, depth + 1));
                }
            }
        }
    }
    out.sort_by(|a, b| (&a.1.asset.source, &a.1.asset.id).cmp(&(&b.1.asset.source, &b.1.asset.id)));
    Ok(out)
}

#[cfg(test)]
pub(crate) fn sample() -> Record {
    Record {
        asset: Asset {
            source: "polyhaven".into(),
            id: "sky_01".into(),
            name: "Sky 01".into(),
            kind: Kind::Hdri,
            author: Some("A. Author".into()),
            licence: "CC0-1.0".into(),
            credit: "\"Sky 01\" by A. Author is licensed under CC0 1.0".into(),
            url: Some("https://example.org/a/sky_01".into()),
        },
        files: BTreeMap::from([("sky_01_1k.hdr".to_string(), "ab".repeat(32))]),
        archive: None,
        pfx: "0.0.0".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_record_keeps_its_licence_facts_in_a_pfx() {
        let root = crate::mock::scratch("record");
        let dir = root.join("sky");
        std::fs::create_dir_all(&dir).unwrap();
        let mut record = sample();
        record.archive = Some(("pack.zip".into(), "cd".repeat(32)));
        record.write(&dir).unwrap();
        let bytes = std::fs::read(dir.join(RECORD)).unwrap();
        assert_eq!(&bytes[..8], &pfx_file::MAGIC);
        assert_eq!(Record::read(&dir).unwrap(), record);
        let facts = record.facts();
        let table: Table = toml::from_str(&facts).unwrap();
        for key in ["licence", "credit", "author", "source"] {
            assert!(table.get(key).is_some(), "{key} is where credits read it");
        }
        let listed = list(&root).unwrap();
        assert_eq!(listed, vec![(dir, record)]);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
