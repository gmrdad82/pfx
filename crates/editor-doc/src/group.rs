use std::path::PathBuf;

use crate::edit::{Caret, Edit};
use crate::error::DocError;
use crate::files::Files;

#[derive(Debug)]
pub struct Group {
    label: String,
    edits: Vec<Box<dyn Edit>>,
}

impl Group {
    pub fn new(label: impl Into<String>) -> Group {
        Group {
            label: label.into(),
            edits: Vec::new(),
        }
    }

    pub fn of(label: impl Into<String>, edits: Vec<Box<dyn Edit>>) -> Group {
        Group {
            label: label.into(),
            edits,
        }
    }

    pub fn with(mut self, edit: impl Edit) -> Group {
        self.edits.push(Box::new(edit));
        self
    }

    pub fn push(&mut self, edit: Box<dyn Edit>) {
        self.edits.push(edit);
    }

    pub fn edits(&self) -> &[Box<dyn Edit>] {
        &self.edits
    }

    pub fn len(&self) -> usize {
        self.edits.len()
    }

    pub fn is_empty(&self) -> bool {
        self.edits.is_empty()
    }
}

impl Edit for Group {
    fn label(&self) -> &str {
        &self.label
    }

    fn files(&self) -> Vec<PathBuf> {
        let mut files: Vec<PathBuf> = self.edits.iter().flat_map(|edit| edit.files()).collect();
        files.sort();
        files.dedup();
        files
    }

    fn apply(&mut self, files: &mut Files) -> Result<(), DocError> {
        let touched = self.files();
        let edits = &mut self.edits;
        files.attempt(&touched, |files| {
            edits.iter_mut().try_for_each(|edit| edit.apply(files))
        })
    }

    fn inverse(&self) -> Box<dyn Edit> {
        Box::new(Group {
            label: self.label.clone(),
            edits: self.edits.iter().rev().map(|edit| edit.inverse()).collect(),
        })
    }

    fn caret(&self) -> Option<Caret> {
        self.edits.iter().rev().find_map(|edit| edit.caret())
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::patch::{TomlPatch, parse_path};
    use crate::splice::TextSplice;
    use toml_edit::value;

    #[test]
    fn a_group_applies_as_one_and_inverts_in_reverse() {
        let mut files = Files::new();
        files.open("a.toml", "x = 1\n");
        files.open("b.txt", "hello\n");
        let mut group = Group::new("drag")
            .with(TomlPatch::set("a.toml", parse_path("x").unwrap(), value(2)))
            .with(TextSplice::new("b.txt", 0..5, "hello", "bye"))
            .with(TomlPatch::set("a.toml", parse_path("y").unwrap(), value(3)));
        assert_eq!(group.files(), vec![PathBuf::from("a.toml"), "b.txt".into()]);
        group.apply(&mut files).unwrap();
        assert_eq!(files.text(Path::new("a.toml")), Some("x = 2\ny = 3\n"));
        assert_eq!(files.text(Path::new("b.txt")), Some("bye\n"));
        assert_eq!(group.caret().unwrap().selection, 3..3);
        group.inverse().apply(&mut files).unwrap();
        assert_eq!(files.text(Path::new("a.toml")), Some("x = 1\n"));
        assert_eq!(files.text(Path::new("b.txt")), Some("hello\n"));
    }

    #[test]
    fn a_refused_member_leaves_every_file_as_it_was() {
        let mut files = Files::new();
        files.open("a.toml", "x = 1\n");
        files.open("b.txt", "hello\n");
        let generation = files.generation();
        let mut group = Group::new("paste")
            .with(TomlPatch::set("a.toml", parse_path("x").unwrap(), value(2)))
            .with(TextSplice::new("b.txt", 0..5, "other", "bye"));
        assert!(group.apply(&mut files).is_err());
        assert_eq!(files.text(Path::new("a.toml")), Some("x = 1\n"));
        assert_eq!(files.text(Path::new("b.txt")), Some("hello\n"));
        assert_eq!(files.generation(), generation);
    }
}
