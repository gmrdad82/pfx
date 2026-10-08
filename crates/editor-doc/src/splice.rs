use std::ops::Range;
use std::path::{Path, PathBuf};

use crate::edit::{Caret, Edit, downcast};
use crate::error::DocError;
use crate::files::{Files, check_range};

pub const TYPING: &str = "typing";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextSplice {
    pub label: String,
    pub file: PathBuf,
    pub range: Range<usize>,
    pub old: String,
    pub new: String,
    pub caret_before: Range<usize>,
    pub caret_after: Range<usize>,
}

pub(crate) fn diff(before: &str, after: &str) -> (usize, usize, usize) {
    let (a, b) = (before.as_bytes(), after.as_bytes());
    let mut prefix = a.iter().zip(b).take_while(|(x, y)| x == y).count();
    while !before.is_char_boundary(prefix) || !after.is_char_boundary(prefix) {
        prefix -= 1;
    }
    let room = a.len().min(b.len()) - prefix;
    let mut suffix = a
        .iter()
        .rev()
        .zip(b.iter().rev())
        .take(room)
        .take_while(|(x, y)| x == y)
        .count();
    while !before.is_char_boundary(a.len() - suffix) || !after.is_char_boundary(b.len() - suffix) {
        suffix -= 1;
    }
    (prefix, a.len() - suffix, b.len() - suffix)
}

impl TextSplice {
    pub fn new(
        file: impl Into<PathBuf>,
        range: Range<usize>,
        old: impl Into<String>,
        new: impl Into<String>,
    ) -> TextSplice {
        let new = new.into();
        let end = range.start + new.len();
        TextSplice {
            label: TYPING.to_string(),
            file: file.into(),
            caret_before: range.clone(),
            range,
            old: old.into(),
            new,
            caret_after: end..end,
        }
    }

    pub fn between(file: impl Into<PathBuf>, before: &str, after: &str) -> Option<TextSplice> {
        if before == after {
            return None;
        }
        let (start, old_end, new_end) = diff(before, after);
        Some(TextSplice::new(
            file,
            start..old_end,
            &before[start..old_end],
            &after[start..new_end],
        ))
    }

    pub fn labelled(mut self, label: impl Into<String>) -> TextSplice {
        self.label = label.into();
        self
    }

    pub fn with_carets(mut self, before: Range<usize>, after: Range<usize>) -> TextSplice {
        self.caret_before = before;
        self.caret_after = after;
        self
    }

    pub fn file(&self) -> &Path {
        &self.file
    }

    pub fn end(&self) -> usize {
        self.range.start + self.new.len()
    }

    fn then(&self, next: &TextSplice) -> Option<TextSplice> {
        if next.file != self.file {
            return None;
        }
        let (s, e) = (self.range.start, self.end());
        let (a, b) = (next.range.start, next.range.end);
        if a > e || b < s {
            return None;
        }
        let low = s.min(a);
        let mut old = String::new();
        if a < s {
            old.push_str(next.old.get(..s - a)?);
        }
        old.push_str(&self.old);
        if b > e {
            old.push_str(next.old.get(e - a..)?);
        }
        let mut new = String::new();
        if s < a {
            new.push_str(self.new.get(..a - s)?);
        }
        new.push_str(&next.new);
        if b < e {
            new.push_str(self.new.get(b - s..)?);
        }
        Some(TextSplice {
            label: self.label.clone(),
            file: self.file.clone(),
            range: low..low + old.len(),
            old,
            new,
            caret_before: self.caret_before.clone(),
            caret_after: next.caret_after.clone(),
        })
    }
}

impl Edit for TextSplice {
    fn label(&self) -> &str {
        &self.label
    }

    fn files(&self) -> Vec<PathBuf> {
        vec![self.file.clone()]
    }

    fn apply(&mut self, files: &mut Files) -> Result<(), DocError> {
        let text = files
            .text(&self.file)
            .ok_or_else(|| DocError::new(&self.file, "", "the file is not open"))?;
        check_range(&self.file, text, &self.range)?;
        if text[self.range.clone()] != self.old {
            return Err(DocError::new(
                &self.file,
                "",
                format!(
                    "bytes {}..{} no longer hold the text this edit replaces",
                    self.range.start, self.range.end
                ),
            ));
        }
        files.splice(&self.file, self.range.clone(), &self.new)?;
        Ok(())
    }

    fn inverse(&self) -> Box<dyn Edit> {
        Box::new(TextSplice {
            label: self.label.clone(),
            file: self.file.clone(),
            range: self.range.start..self.end(),
            old: self.new.clone(),
            new: self.old.clone(),
            caret_before: self.caret_after.clone(),
            caret_after: self.caret_before.clone(),
        })
    }

    fn merge(&self, next: &dyn Edit) -> Option<Box<dyn Edit>> {
        let next = downcast::<TextSplice>(next)?;
        self.then(next)
            .map(|merged| Box::new(merged) as Box<dyn Edit>)
    }

    fn caret(&self) -> Option<Caret> {
        Some(Caret {
            file: self.file.clone(),
            selection: self.caret_after.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = "a.toml";

    fn splice(range: Range<usize>, old: &str, new: &str) -> TextSplice {
        TextSplice::new(FILE, range, old, new)
    }

    fn run(text: &str, splices: &[TextSplice]) -> String {
        let mut files = Files::new();
        files.open(FILE, text);
        for s in splices {
            s.clone().apply(&mut files).unwrap();
        }
        files.text(Path::new(FILE)).unwrap().to_string()
    }

    #[test]
    fn between_finds_the_smallest_change() {
        let s = TextSplice::between(FILE, "x = 1.0\n", "x = 12.0\n").unwrap();
        assert_eq!(
            (s.range.clone(), s.old.as_str(), s.new.as_str()),
            (5..5, "", "2")
        );
        let s = TextSplice::between(FILE, "aaa", "aa").unwrap();
        assert_eq!(
            (s.range.clone(), s.old.as_str(), s.new.as_str()),
            (2..3, "a", "")
        );
        let s = TextSplice::between(FILE, "ä", "ö").unwrap();
        assert_eq!((s.old.as_str(), s.new.as_str()), ("ä", "ö"));
        assert!(TextSplice::between(FILE, "same", "same").is_none());
    }

    #[test]
    fn a_splice_and_its_inverse_restore_the_text() {
        let text = "name = \"ä\"\nat = [1.0]\n";
        let mut s = splice(8..10, "ä", "öö");
        let mut files = Files::new();
        files.open(FILE, text);
        s.apply(&mut files).unwrap();
        assert_eq!(
            files.text(Path::new(FILE)),
            Some("name = \"öö\"\nat = [1.0]\n")
        );
        s.inverse().apply(&mut files).unwrap();
        assert_eq!(files.text(Path::new(FILE)), Some(text));
    }

    #[test]
    fn a_splice_over_other_text_is_refused() {
        let mut files = Files::new();
        files.open(FILE, "x = 1\n");
        assert!(splice(4..5, "2", "3").apply(&mut files).is_err());
        assert!(splice(4..9, "1", "3").apply(&mut files).is_err());
        assert_eq!(files.text(Path::new(FILE)), Some("x = 1\n"));
    }

    #[test]
    fn composed_splices_equal_the_pair() {
        let text = "abcdef";
        let first = splice(2..3, "c", "XYZ");
        let cases = [
            splice(5..5, "", "!"),
            splice(3..4, "Y", ""),
            splice(1..3, "bX", "_"),
            splice(4..7, "Zde", "-"),
            splice(0..8, "abXYZdef", ""),
            splice(2..2, "", "<"),
        ];
        for next in cases {
            let merged = first.then(&next).unwrap();
            assert_eq!(run(text, &[merged]), run(text, &[first.clone(), next]));
        }
        assert!(first.then(&splice(6..6, "", "?")).is_none());
        assert!(first.then(&splice(0..1, "a", "")).is_none());
    }

    #[test]
    fn typing_merges_into_one_splice_with_the_first_caret() {
        let a = splice(0..0, "", "a").with_carets(0..0, 1..1);
        let b = splice(1..1, "", "b").with_carets(1..1, 2..2);
        let merged = a.merge(&b).unwrap();
        let merged = downcast::<TextSplice>(merged.as_ref()).unwrap();
        assert_eq!(merged.new, "ab");
        assert_eq!(merged.caret_before, 0..0);
        assert_eq!(merged.caret_after, 2..2);
        assert_eq!(merged.inverse().caret().unwrap().selection, 0..0);
    }
}
