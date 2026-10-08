use std::path::PathBuf;

use crate::edit::{Caret, Edit};
use crate::error::DocError;
use crate::files::Files;

pub const DEPTH: usize = 256;
pub const GAP: f64 = 1.0;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Changed {
    pub label: String,
    pub files: Vec<PathBuf>,
    pub caret: Option<Caret>,
}

impl Changed {
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }
}

#[derive(Debug)]
pub struct History {
    undo: Vec<Box<dyn Edit>>,
    redo: Vec<Box<dyn Edit>>,
    depth: usize,
    typed_at: Option<f64>,
}

impl Default for History {
    fn default() -> History {
        History::new(DEPTH)
    }
}

pub fn run(files: &mut Files, edit: &mut dyn Edit) -> Result<Changed, DocError> {
    let touched = edit.files();
    let before: Vec<Option<u64>> = touched.iter().map(|p| files.generation_of(p)).collect();
    files.attempt(&touched, |files| edit.apply(files))?;
    let changed = touched
        .into_iter()
        .zip(before)
        .filter(|(path, generation)| files.generation_of(path) != *generation)
        .map(|(path, _)| path)
        .collect();
    Ok(Changed {
        label: edit.label().to_string(),
        files: changed,
        caret: edit.caret(),
    })
}

fn keep(entries: &mut Vec<Box<dyn Edit>>, files: &Files) -> usize {
    let mut scratch = files.clone();
    let before = entries.len();
    let mut kept: Vec<Box<dyn Edit>> = entries
        .drain(..)
        .rev()
        .filter(|entry| run(&mut scratch, entry.inverse().as_mut()).is_ok())
        .collect();
    kept.reverse();
    *entries = kept;
    before - entries.len()
}

impl History {
    pub fn new(depth: usize) -> History {
        History {
            undo: Vec::new(),
            redo: Vec::new(),
            depth: depth.max(1),
            typed_at: None,
        }
    }

    pub fn depth(&self) -> usize {
        self.depth
    }

    pub fn set_depth(&mut self, depth: usize) {
        self.depth = depth.max(1);
        self.trim();
    }

    fn trim(&mut self) {
        if self.undo.len() > self.depth {
            let extra = self.undo.len() - self.depth;
            self.undo.drain(..extra);
        }
    }

    pub fn apply(&mut self, files: &mut Files, edit: impl Edit) -> Result<Changed, DocError> {
        self.record(files, Box::new(edit), None)
    }

    pub fn apply_boxed(
        &mut self,
        files: &mut Files,
        edit: Box<dyn Edit>,
    ) -> Result<Changed, DocError> {
        self.record(files, edit, None)
    }

    pub fn typed(
        &mut self,
        files: &mut Files,
        edit: impl Edit,
        now: f64,
        gap: f64,
    ) -> Result<Changed, DocError> {
        self.record(files, Box::new(edit), Some((now, gap)))
    }

    fn record(
        &mut self,
        files: &mut Files,
        mut edit: Box<dyn Edit>,
        typed: Option<(f64, f64)>,
    ) -> Result<Changed, DocError> {
        let changed = run(files, edit.as_mut())?;
        if changed.is_empty() {
            return Ok(changed);
        }
        self.redo.clear();
        let merged = match (typed, self.typed_at, self.undo.last()) {
            (Some((now, gap)), Some(at), Some(top)) if now >= at && now - at <= gap => {
                top.merge(edit.as_ref())
            }
            _ => None,
        };
        match merged {
            Some(merged) => {
                let top = self.undo.len() - 1;
                self.undo[top] = merged;
            }
            None => {
                self.undo.push(edit);
                self.trim();
            }
        }
        self.typed_at = typed.map(|(now, _)| now);
        Ok(changed)
    }

    pub fn undo(&mut self, files: &mut Files) -> Result<Option<Changed>, DocError> {
        self.step(files, true)
    }

    pub fn redo(&mut self, files: &mut Files) -> Result<Option<Changed>, DocError> {
        self.step(files, false)
    }

    fn step(&mut self, files: &mut Files, undo: bool) -> Result<Option<Changed>, DocError> {
        let (from, to) = if undo {
            (&mut self.undo, &mut self.redo)
        } else {
            (&mut self.redo, &mut self.undo)
        };
        let Some(top) = from.last() else {
            return Ok(None);
        };
        let mut back = top.inverse();
        let changed = run(files, back.as_mut())?;
        from.pop();
        to.push(back);
        self.typed_at = None;
        self.trim();
        Ok(Some(changed))
    }

    pub fn seal(&mut self) {
        self.typed_at = None;
    }

    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.typed_at = None;
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo_len(&self) -> usize {
        self.undo.len()
    }

    pub fn redo_len(&self) -> usize {
        self.redo.len()
    }

    pub fn undo_label(&self) -> Option<&str> {
        self.undo.last().map(|edit| edit.label())
    }

    pub fn redo_label(&self) -> Option<&str> {
        self.redo.last().map(|edit| edit.label())
    }

    pub fn undo_labels(&self) -> Vec<&str> {
        self.undo.iter().map(|edit| edit.label()).collect()
    }

    pub fn redo_labels(&self) -> Vec<&str> {
        self.redo.iter().map(|edit| edit.label()).collect()
    }

    pub fn prune(&mut self, files: &Files) -> usize {
        self.typed_at = None;
        keep(&mut self.undo, files) + keep(&mut self.redo, files)
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::patch::{TomlPatch, parse_path};
    use crate::splice::TextSplice;
    use toml_edit::value;

    const FILE: &str = "a.toml";

    fn files(text: &str) -> Files {
        let mut files = Files::new();
        files.open(FILE, text);
        files
    }

    fn text(files: &Files) -> &str {
        files.text(Path::new(FILE)).unwrap()
    }

    fn set(key: &str, n: i64) -> TomlPatch {
        TomlPatch::set(FILE, parse_path(key).unwrap(), value(n)).labelled(key)
    }

    fn typed(at: usize, what: &str) -> TextSplice {
        TextSplice::new(FILE, at..at, "", what)
            .with_carets(at..at, at + what.len()..at + what.len())
    }

    #[test]
    fn a_new_step_clears_the_redo_side() {
        let mut f = files("a = 0\n");
        let mut history = History::default();
        history.apply(&mut f, set("one", 1)).unwrap();
        history.apply(&mut f, set("two", 2)).unwrap();
        history.undo(&mut f).unwrap().unwrap();
        assert_eq!(
            (history.undo_labels(), history.redo_labels()),
            (vec!["one"], vec!["two"])
        );
        assert_eq!(history.redo_label(), Some("two"));
        history.apply(&mut f, set("three", 3)).unwrap();
        assert_eq!(
            (history.undo_labels(), history.redo_labels()),
            (vec!["one", "three"], vec![])
        );
    }

    #[test]
    fn undo_and_redo_say_what_changed() {
        let mut f = files("a = 0\n");
        let mut history = History::default();
        history.apply(&mut f, set("a", 1)).unwrap();
        let changed = history.undo(&mut f).unwrap().unwrap();
        assert_eq!(changed.label, "a");
        assert_eq!(changed.files, vec![PathBuf::from(FILE)]);
        assert_eq!(text(&f), "a = 0\n");
        assert!(history.undo(&mut f).unwrap().is_none());
        history.redo(&mut f).unwrap().unwrap();
        assert_eq!(text(&f), "a = 1\n");
        assert!(history.redo(&mut f).unwrap().is_none());
    }

    #[test]
    fn the_stack_keeps_its_last_steps() {
        let mut f = files("a = 0\n");
        let mut history = History::new(8);
        for step in 1..=11 {
            history
                .apply(&mut f, set("a", step).labelled(step.to_string()))
                .unwrap();
        }
        assert_eq!(history.undo_len(), 8);
        assert_eq!(history.undo_labels()[0], "4");
        history.set_depth(2);
        assert_eq!(history.undo_labels(), vec!["10", "11"]);
    }

    #[test]
    fn a_refused_edit_records_nothing_and_a_no_op_is_not_a_step() {
        let mut f = files("a = 0\n");
        let mut history = History::default();
        let generation = f.generation();
        assert!(
            history
                .apply(&mut f, TomlPatch::unset(FILE, parse_path("b").unwrap()))
                .is_err()
        );
        assert!(history.apply(&mut f, set("a", 0)).unwrap().is_empty());
        assert_eq!(history.undo_len(), 0);
        assert_eq!(f.generation(), generation);
    }

    #[test]
    fn typing_merges_within_the_gap_and_splits_across_it_or_a_jump() {
        let mut f = files("a = 0\n");
        let mut history = History::default();
        history.typed(&mut f, typed(0, "#"), 0.0, GAP).unwrap();
        history.typed(&mut f, typed(1, " "), 0.5, GAP).unwrap();
        history.typed(&mut f, typed(2, "x"), 1.4, GAP).unwrap();
        assert_eq!(history.undo_len(), 1);
        history.typed(&mut f, typed(3, "y"), 3.0, GAP).unwrap();
        assert_eq!(history.undo_len(), 2);
        history.typed(&mut f, typed(0, "z"), 3.1, GAP).unwrap();
        assert_eq!(history.undo_len(), 3);
        assert_eq!(text(&f), "z# xya = 0\n");
        let changed = history.undo(&mut f).unwrap().unwrap();
        assert_eq!(changed.caret.unwrap().selection, 0..0);
        history.undo(&mut f).unwrap();
        assert_eq!(text(&f), "# xa = 0\n");
        let changed = history.undo(&mut f).unwrap().unwrap();
        assert_eq!(changed.caret.unwrap().selection, 0..0);
        assert_eq!(text(&f), "a = 0\n");
    }

    #[test]
    fn a_patch_or_an_undo_ends_the_typing_run() {
        let mut f = files("a = 0\n");
        let mut history = History::default();
        history.typed(&mut f, typed(0, "#"), 0.0, GAP).unwrap();
        history.apply(&mut f, set("b", 1)).unwrap();
        history.typed(&mut f, typed(1, " "), 0.1, GAP).unwrap();
        assert_eq!(history.undo_len(), 3);
        history.undo(&mut f).unwrap();
        history.typed(&mut f, typed(1, "!"), 0.2, GAP).unwrap();
        assert_eq!(history.undo_len(), 3);
        history.seal();
        history.typed(&mut f, typed(2, "!"), 0.3, GAP).unwrap();
        assert_eq!(history.undo_len(), 4);
    }

    #[test]
    fn pruning_drops_only_the_steps_that_no_longer_apply() {
        let mut f = files("a = 0\n[t]\nx = 0\n");
        let mut history = History::default();
        history.apply(&mut f, set("t.x", 1)).unwrap();
        history.typed(&mut f, typed(0, "# x\n"), 0.0, GAP).unwrap();
        history.apply(&mut f, set("a", 1)).unwrap();
        f.reload(Path::new(FILE), "a = 1\n").unwrap();
        assert_eq!(history.prune(&f), 2);
        assert_eq!(history.undo_labels(), vec!["a"]);
        history.undo(&mut f).unwrap();
        assert_eq!(text(&f), "a = 0\n");
    }
}
