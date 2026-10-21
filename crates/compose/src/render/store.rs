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
    packed: bool,
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

    pub fn style_file(path: &Path) -> Result<Document, String> {
        if path.extension().is_none_or(|e| e != "toml") {
            return Document::open(path);
        }
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let text = String::from_utf8(bytes)
            .map_err(|_| format!("{}: the file is not UTF-8", path.display()))?;
        Ok(Document {
            path: path.to_path_buf(),
            bytes: Vec::new(),
            text,
            tag: pfx_file::SCNE,
            name: String::new(),
            packed: false,
        })
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
            packed: true,
        })
    }

    pub fn file(&self, name: &str) -> Result<Option<Vec<u8>>, String> {
        self.file_within(name, u64::MAX)
    }

    pub fn sample(&self, name: &str) -> Result<Option<Vec<u8>>, String> {
        self.file_within(name, super::bounds::MAX_SAMPLE_BYTES)
    }

    pub fn file_within(&self, name: &str, most: u64) -> Result<Option<Vec<u8>>, String> {
        if !self.packed {
            return Ok(None);
        }
        let shown = self.path.display();
        let reader = Reader::from_bytes(&self.bytes).map_err(|e| format!("{shown}: {e}"))?;
        if let Some(index) = reader.find_named(pfx_file::FILE, name) {
            let raw = reader.entries()[index].raw;
            if raw > most {
                return Err(format!(
                    "{shown}: `{name}` holds {raw} bytes; the render reads at most {most}"
                ));
            }
        }
        reader
            .named(pfx_file::FILE, name)
            .map(|found| found.map(std::borrow::Cow::into_owned))
            .map_err(|e| format!("{shown}: {e}"))
    }

    pub fn holds(&self, name: &str) -> Result<bool, String> {
        let shown = self.path.display();
        let reader = Reader::from_bytes(&self.bytes).map_err(|e| format!("{shown}: {e}"))?;
        Ok(reader.find_named(pfx_file::FILE, name).is_some())
    }

    pub fn holds_audio(&self, written: &[String]) -> Result<bool, String> {
        let shown = self.path.display();
        let reader = Reader::from_bytes(&self.bytes).map_err(|e| format!("{shown}: {e}"))?;
        Ok(reader
            .entries()
            .iter()
            .any(|e| e.tag == pfx_file::FILE && written.contains(&e.name)))
    }

    pub fn save(&self, text: &str, written: &[String]) -> Result<(), String> {
        let shown = self.path.display();
        let reader = Reader::from_bytes(&self.bytes).map_err(|e| format!("{shown}: {e}"))?;
        let mut writer = Writer::new();
        for (index, entry) in reader.entries().iter().enumerate() {
            if entry.tag == self.tag && entry.name == self.name {
                writer = writer.named(self.tag, &self.name, text.as_bytes(), Codec::Auto);
                continue;
            }
            let stale = entry.tag == pfx_file::FILE && written.contains(&entry.name);
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

pub fn linked(folder: &Path, file: &str) -> Result<Document, String> {
    Document::style_file(&beside(folder, file)?)
}

pub fn beside(folder: &Path, file: &str) -> Result<PathBuf, String> {
    let relative = Path::new(file);
    if relative.is_absolute()
        || relative
            .components()
            .any(|part| !matches!(part, std::path::Component::Normal(_)))
    {
        return Err(format!(
            "`{file}` must name a file in the document's folder or below it"
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
    fn saving_a_score_keeps_every_other_chunk_and_drops_only_the_audio_it_wrote() {
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
                "compose/a/mine.wav",
                b"mine".as_slice(),
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
        doc.save(&text, &["compose/a/old.wav".to_string()]).unwrap();
        let again = Document::open(&path).unwrap();
        assert!(again.text.contains("[music]\nbpm = 120"), "{}", again.text);
        assert_eq!(again.file("keep.bin").unwrap().unwrap(), b"keep");
        assert_eq!(again.file("compose/a/old.wav").unwrap(), None);
        assert_eq!(again.file("compose/a/mine.wav").unwrap().unwrap(), b"mine");
        assert_eq!(
            again.file("compose/b/other.wav").unwrap().unwrap(),
            b"other"
        );
    }
}
