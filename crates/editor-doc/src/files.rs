use std::collections::BTreeMap;
use std::io::ErrorKind;
use std::ops::Range;
use std::path::{Path, PathBuf};

use toml_edit::{DocumentMut, TomlError};

use crate::error::DocError;
use crate::stamp::{Disk, Own, Seen, Stamp};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Toml,
    Text,
}

impl Format {
    pub fn of(path: &Path) -> Format {
        match path.extension().and_then(|ext| ext.to_str()) {
            Some(ext) if ext.eq_ignore_ascii_case("toml") => Format::Toml,
            _ => Format::Text,
        }
    }
}

#[derive(Clone, Debug)]
pub struct File {
    format: Format,
    text: String,
    doc: Option<DocumentMut>,
    error: Option<TomlError>,
    generation: u64,
    own: Own,
}

impl File {
    fn new(format: Format, text: String, generation: u64) -> File {
        let mut file = File {
            format,
            text: String::new(),
            doc: None,
            error: None,
            generation,
            own: Own::default(),
        };
        file.own.read(&text);
        file.set(text);
        file
    }

    fn set(&mut self, text: String) {
        self.text = text;
        if self.format == Format::Toml {
            match self.text.parse::<DocumentMut>() {
                Ok(doc) => {
                    self.doc = Some(doc);
                    self.error = None;
                }
                Err(error) => {
                    self.doc = None;
                    self.error = Some(error);
                }
            }
        }
    }

    pub fn format(&self) -> Format {
        self.format
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn doc(&self) -> Option<&DocumentMut> {
        self.doc.as_ref()
    }

    pub fn error(&self) -> Option<&TomlError> {
        self.error.as_ref()
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn disk_stamp(&self) -> Option<Stamp> {
        self.own.current()
    }

    pub fn dirty(&self) -> bool {
        self.own.current() != Some(Stamp::of(&self.text))
    }
}

#[derive(Clone, Debug)]
struct Snapshot {
    generation: u64,
    files: Vec<(PathBuf, Option<File>)>,
}

#[derive(Clone, Debug, Default)]
pub struct Files {
    files: BTreeMap<PathBuf, File>,
    generation: u64,
}

fn read_error(path: &Path, what: &str, error: std::io::Error) -> DocError {
    DocError::new(path, "", format!("cannot {what}: {error}"))
}

fn floor(text: &str, byte: usize) -> usize {
    let mut byte = byte.min(text.len());
    while !text.is_char_boundary(byte) {
        byte -= 1;
    }
    byte
}

pub(crate) fn check_range(path: &Path, text: &str, range: &Range<usize>) -> Result<(), DocError> {
    if range.start > range.end || range.end > text.len() {
        return Err(DocError::new(
            path,
            "",
            format!(
                "bytes {}..{} are outside the text, which has {}",
                range.start,
                range.end,
                text.len()
            ),
        ));
    }
    if floor(text, range.start) != range.start || floor(text, range.end) != range.end {
        return Err(DocError::new(
            path,
            "",
            format!("bytes {}..{} split a character", range.start, range.end),
        ));
    }
    Ok(())
}

impl Files {
    pub fn new() -> Files {
        Files::default()
    }

    fn bump(&mut self) -> u64 {
        self.generation += 1;
        self.generation
    }

    pub fn open(&mut self, path: impl Into<PathBuf>, text: impl Into<String>) -> &File {
        let path = path.into();
        let format = Format::of(&path);
        self.open_as(path, text, format)
    }

    pub fn open_as(
        &mut self,
        path: impl Into<PathBuf>,
        text: impl Into<String>,
        format: Format,
    ) -> &File {
        let generation = self.bump();
        let path = path.into();
        self.files
            .insert(path.clone(), File::new(format, text.into(), generation));
        &self.files[&path]
    }

    pub fn load(&mut self, path: impl Into<PathBuf>) -> Result<&File, DocError> {
        let path = path.into();
        let text =
            std::fs::read_to_string(&path).map_err(|error| read_error(&path, "read", error))?;
        Ok(self.open(path, text))
    }

    pub fn close(&mut self, path: &Path) -> Option<File> {
        let file = self.files.remove(path)?;
        self.bump();
        Some(file)
    }

    pub fn get(&self, path: &Path) -> Option<&File> {
        self.files.get(path)
    }

    pub fn text(&self, path: &Path) -> Option<&str> {
        self.get(path).map(File::text)
    }

    pub fn doc(&self, path: &Path) -> Option<&DocumentMut> {
        self.get(path).and_then(File::doc)
    }

    pub fn paths(&self) -> impl Iterator<Item = &Path> {
        self.files.keys().map(PathBuf::as_path)
    }

    pub fn len(&self) -> usize {
        self.files.len()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn generation_of(&self, path: &Path) -> Option<u64> {
        self.get(path).map(File::generation)
    }

    fn file(&self, path: &Path) -> Result<&File, DocError> {
        self.files
            .get(path)
            .ok_or_else(|| DocError::new(path, "", "the file is not open"))
    }

    pub fn set_text(&mut self, path: &Path, text: impl Into<String>) -> Result<bool, DocError> {
        let text = text.into();
        if self.file(path)?.text == text {
            return Ok(false);
        }
        let generation = self.bump();
        let file = self.files.get_mut(path).expect("the file is open");
        file.set(text);
        file.generation = generation;
        Ok(true)
    }

    pub(crate) fn set_parsed(
        &mut self,
        path: &Path,
        text: String,
        doc: DocumentMut,
    ) -> Result<bool, DocError> {
        let file = self.file(path)?;
        if file.text == text {
            return Ok(false);
        }
        if file.format != Format::Toml {
            return self.set_text(path, text);
        }
        let generation = self.bump();
        let file = self.files.get_mut(path).expect("the file is open");
        file.text = text;
        file.doc = Some(doc);
        file.error = None;
        file.generation = generation;
        Ok(true)
    }

    pub fn splice(
        &mut self,
        path: &Path,
        range: Range<usize>,
        new: &str,
    ) -> Result<bool, DocError> {
        let file = self.file(path)?;
        check_range(path, &file.text, &range)?;
        if &file.text[range.clone()] == new {
            return Ok(false);
        }
        let mut text = String::with_capacity(file.text.len() - range.len() + new.len());
        text.push_str(&file.text[..range.start]);
        text.push_str(new);
        text.push_str(&file.text[range.end..]);
        self.set_text(path, text)
    }

    pub fn save(&mut self, path: &Path) -> Result<Stamp, DocError> {
        let file = self.file(path)?;
        let name = path
            .file_name()
            .map_or_else(|| "file".to_string(), |n| n.to_string_lossy().into_owned());
        let temp = path.with_file_name(format!(".{name}.save-tmp"));
        std::fs::write(&temp, &file.text).map_err(|error| read_error(path, "write", error))?;
        std::fs::rename(&temp, path).map_err(|error| {
            let _ = std::fs::remove_file(&temp);
            read_error(path, "replace", error)
        })?;
        let file = self.files.get_mut(path).expect("the file is open");
        Ok(file.own.wrote(&file.text))
    }

    pub fn seen(&self, path: &Path, stamp: Stamp) -> Option<Seen> {
        self.get(path).map(|file| file.own.seen(stamp))
    }

    pub fn disk(&self, path: &Path) -> Result<Disk, DocError> {
        let file = self.file(path)?;
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Disk::Missing),
            Err(error) => return Err(read_error(path, "read", error)),
        };
        Ok(match file.own.seen(Stamp::of(&text)) {
            Seen::Current => Disk::Same,
            Seen::Earlier => Disk::Own,
            Seen::Outside => Disk::Outside(text),
        })
    }

    pub fn reload(&mut self, path: &Path, text: impl Into<String>) -> Result<bool, DocError> {
        let text = text.into();
        let changed = self.set_text(path, text.clone())?;
        self.files
            .get_mut(path)
            .expect("the file is open")
            .own
            .read(&text);
        Ok(changed)
    }

    fn snapshot(&self, paths: &[PathBuf]) -> Snapshot {
        Snapshot {
            generation: self.generation,
            files: paths
                .iter()
                .map(|path| (path.clone(), self.files.get(path).cloned()))
                .collect(),
        }
    }

    fn restore(&mut self, snapshot: Snapshot) {
        self.generation = snapshot.generation;
        for (path, file) in snapshot.files {
            match file {
                Some(file) => {
                    self.files.insert(path, file);
                }
                None => {
                    self.files.remove(&path);
                }
            }
        }
    }

    pub fn attempt<T>(
        &mut self,
        paths: &[PathBuf],
        change: impl FnOnce(&mut Files) -> Result<T, DocError>,
    ) -> Result<T, DocError> {
        let snapshot = self.snapshot(paths);
        change(self).inspect_err(|_| self.restore(snapshot))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(name: &str) -> PathBuf {
        PathBuf::from(name)
    }

    #[test]
    fn a_toml_file_keeps_a_document_in_step() {
        let mut files = Files::new();
        files.open("a.toml", "x = 1\n");
        assert_eq!(
            files.doc(&path("a.toml")).unwrap()["x"].as_integer(),
            Some(1)
        );
        files.set_text(&path("a.toml"), "x = 2\n").unwrap();
        assert_eq!(
            files.doc(&path("a.toml")).unwrap()["x"].as_integer(),
            Some(2)
        );
        files.set_text(&path("a.toml"), "x = \n").unwrap();
        assert!(files.doc(&path("a.toml")).is_none());
        assert!(files.get(&path("a.toml")).unwrap().error().is_some());
        files.open("notes.txt", "x = \n");
        assert_eq!(
            files.get(&path("notes.txt")).unwrap().format(),
            Format::Text
        );
        assert!(files.get(&path("notes.txt")).unwrap().error().is_none());
    }

    #[test]
    fn the_generation_moves_only_on_a_real_change() {
        let mut files = Files::new();
        files.open("a.toml", "x = 1\n");
        let a = path("a.toml");
        let before = (files.generation(), files.generation_of(&a));
        assert!(!files.set_text(&a, "x = 1\n").unwrap());
        assert!(!files.splice(&a, 4..5, "1").unwrap());
        assert_eq!((files.generation(), files.generation_of(&a)), before);
        assert!(files.splice(&a, 4..5, "7").unwrap());
        assert_eq!(files.text(&a), Some("x = 7\n"));
        assert!(files.generation() > before.0);
        assert_eq!(files.generation_of(&a), Some(files.generation()));
    }

    #[test]
    fn a_splice_inside_a_character_is_refused() {
        let mut files = Files::new();
        files.open("a.txt", "ä\n");
        let a = path("a.txt");
        assert!(files.splice(&a, 1..2, "x").is_err());
        assert!(files.splice(&a, 0..9, "x").is_err());
        assert!(files.set_text(&path("b.txt"), "x").is_err());
        assert_eq!(files.text(&a), Some("ä\n"));
    }

    #[test]
    fn a_failed_attempt_restores_every_touched_file() {
        let mut files = Files::new();
        files.open("a.toml", "x = 1\n");
        let a = path("a.toml");
        let generation = files.generation();
        let out: Result<(), DocError> = files.attempt(std::slice::from_ref(&a), |files| {
            files.set_text(&a, "x = 2\n")?;
            Err(DocError::new(&a, "", "refused"))
        });
        assert!(out.is_err());
        assert_eq!(files.text(&a), Some("x = 1\n"));
        assert_eq!(files.generation(), generation);
    }

    #[test]
    fn an_edit_makes_a_file_dirty_until_it_reads_its_disk_again() {
        let mut files = Files::new();
        files.open("a.toml", "x = 1\n");
        let a = path("a.toml");
        assert!(!files.get(&a).unwrap().dirty());
        files.set_text(&a, "x = 2\n").unwrap();
        assert!(files.get(&a).unwrap().dirty());
        files.set_text(&a, "x = 1\n").unwrap();
        assert!(!files.get(&a).unwrap().dirty());
        files.reload(&a, "x = 3\n").unwrap();
        assert!(!files.get(&a).unwrap().dirty());
        assert_eq!(files.seen(&a, Stamp::of("x = 3\n")), Some(Seen::Current));
    }
}
