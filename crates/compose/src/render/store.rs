use std::path::{Path, PathBuf};

use pfx_file::{Codec, Reader, Tag, Writer};

pub const FOLDER: &str = "compose";

pub fn stem_file(score: &str, role: &str) -> String {
    format!("{FOLDER}/{score}/{role}.wav")
}

#[derive(Clone, Debug)]
pub struct Document {
    pub path: PathBuf,
    bytes: Vec<u8>,
    pub text: String,
    tag: Tag,
    name: String,
}

fn has_compose(text: &str) -> bool {
    text.parse::<toml::Table>()
        .is_ok_and(|table| table.contains_key("compose"))
}

impl Document {
    pub fn open(path: &Path) -> Result<Document, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Document::from_bytes(path, bytes)
    }

    pub fn from_bytes(path: &Path, bytes: Vec<u8>) -> Result<Document, String> {
        let shown = path.display();
        let reader = Reader::from_bytes(&bytes).map_err(|e| format!("{shown}: {e}"))?;
        let mut first = None;
        let mut found = None;
        for tag in [pfx_file::SCNE, pfx_file::RCPE] {
            for (name, chunk) in reader.chunks(tag).map_err(|e| format!("{shown}: {e}"))? {
                let text = String::from_utf8(chunk.into_owned())
                    .map_err(|_| format!("{shown}: `{name}` is not UTF-8"))?;
                if found.is_none() && has_compose(&text) {
                    found = Some((tag, name.to_string(), text.clone()));
                }
                if first.is_none() {
                    first = Some((tag, name.to_string(), text));
                }
            }
        }
        let (tag, name, text) = found
            .or(first)
            .ok_or_else(|| format!("{shown}: holds no scene or recipe text"))?;
        Ok(Document {
            path: path.to_path_buf(),
            bytes,
            text,
            tag,
            name,
        })
    }

    pub fn file(&self, name: &str) -> Result<Option<Vec<u8>>, String> {
        let shown = self.path.display();
        let reader = Reader::from_bytes(&self.bytes).map_err(|e| format!("{shown}: {e}"))?;
        reader
            .named(pfx_file::FILE, name)
            .map(|found| found.map(std::borrow::Cow::into_owned))
            .map_err(|e| format!("{shown}: {e}"))
    }

    pub fn holds_audio(&self, score: &str) -> Result<bool, String> {
        let shown = self.path.display();
        let reader = Reader::from_bytes(&self.bytes).map_err(|e| format!("{shown}: {e}"))?;
        let folder = format!("{FOLDER}/{score}/");
        Ok(reader
            .entries()
            .iter()
            .any(|e| e.tag == pfx_file::FILE && e.name.starts_with(folder.as_str())))
    }

    pub fn save(&self, text: &str, score: Option<&str>) -> Result<(), String> {
        let shown = self.path.display();
        let reader = Reader::from_bytes(&self.bytes).map_err(|e| format!("{shown}: {e}"))?;
        let dropped = score.map(|score| format!("{FOLDER}/{score}/"));
        let mut writer = Writer::new();
        for (index, entry) in reader.entries().iter().enumerate() {
            if entry.tag == self.tag && entry.name == self.name {
                writer = writer.named(self.tag, &self.name, text.as_bytes(), Codec::Auto);
                continue;
            }
            let stale = entry.tag == pfx_file::FILE
                && dropped
                    .as_ref()
                    .is_some_and(|d| entry.name.starts_with(d.as_str()));
            if stale {
                continue;
            }
            writer = writer
                .copy(&reader, index)
                .map_err(|e| format!("{shown}: {e}"))?;
        }
        writer
            .write(&self.path)
            .map_err(|e| format!("{shown}: {e}"))
    }
}

pub fn beside(folder: &Path, file: &str) -> Result<PathBuf, String> {
    let relative = Path::new(file);
    if relative.is_absolute()
        || relative
            .components()
            .any(|part| !matches!(part, std::path::Component::Normal(_)))
    {
        return Err(format!(
            "`{file}` must name a file in the document's own folder"
        ));
    }
    Ok(folder.join(relative))
}

pub fn set_table(text: &str, key: &str, table: toml::Table) -> Result<String, String> {
    let mut doc: toml_edit::DocumentMut = text.parse().map_err(|e| format!("{e}"))?;
    let rendered = toml::to_string(&toml::Table::from_iter([(
        key.to_string(),
        toml::Value::Table(table),
    )]))
    .map_err(|e| e.to_string())?;
    let fresh: toml_edit::DocumentMut = rendered.parse().map_err(|e| format!("{e}"))?;
    let item = fresh
        .get(key)
        .cloned()
        .ok_or_else(|| format!("[{key}] did not render"))?;
    doc.insert(key, item);
    Ok(doc.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(name: &str) -> PathBuf {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tmp/compose-tests")
            .join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn saving_a_score_keeps_every_other_chunk_and_drops_its_old_audio() {
        let path = folder("store").join("doc.pitofx");
        Writer::new()
            .named(
                pfx_file::SCNE,
                "main.toml",
                b"[sky]\nx = 1\n[compose]\n".as_slice(),
                Codec::Auto,
            )
            .named(pfx_file::FILE, "keep.bin", b"keep".as_slice(), Codec::Auto)
            .named(
                pfx_file::FILE,
                "compose/a/old.wav",
                b"old".as_slice(),
                Codec::Auto,
            )
            .named(
                pfx_file::FILE,
                "compose/b/other.wav",
                b"other".as_slice(),
                Codec::Auto,
            )
            .write(&path)
            .unwrap();
        let doc = Document::open(&path).unwrap();
        assert!(doc.text.contains("[sky]"));
        let text = set_table(
            &doc.text,
            "music",
            toml::Table::from_iter([("bpm".to_string(), toml::Value::Integer(120))]),
        )
        .unwrap();
        assert!(text.starts_with("[sky]\nx = 1\n"), "{text}");
        doc.save(&text, Some("a")).unwrap();
        let again = Document::open(&path).unwrap();
        assert!(again.text.contains("[music]\nbpm = 120"), "{}", again.text);
        assert_eq!(again.file("keep.bin").unwrap().unwrap(), b"keep");
        assert_eq!(again.file("compose/a/old.wav").unwrap(), None);
        assert_eq!(
            again.file("compose/b/other.wav").unwrap().unwrap(),
            b"other"
        );
    }
}
