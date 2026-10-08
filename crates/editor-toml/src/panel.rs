use std::hash::Hash;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use egui::text::{CCursor, CCursorRange, LayoutJob, TextFormat};
use egui::text_edit::TextEditState;
use egui::{
    Align, Align2, Galley, Id, InputState, Key, KeyboardShortcut, Label, Layout, Margin, Modifiers,
    Pos2, Rect, RichText, ScrollArea, Sense, Shape, Stroke, TextBuffer, TextEdit, Ui, Vec2, pos2,
};
use pfx_editor_doc::{Changed, Disk, File, Files, GAP, History, TextSplice, toggle_comment};
use pfx_editor_style::Theme;
use pfx_editor_style::widgets::KeyCap;

use crate::compare::{Basis, Compare};
use crate::diag::{self, Check, Diagnostic, Severity};
use crate::diff::Diff;
use crate::host::{Git, Head, NO_EDITOR, Opener, Process};
use crate::job::JobCache;
use crate::lex::Lexed;
use crate::table::{Table, TableCache};

pub const DEBOUNCE: f64 = 0.15;
pub const PROBLEM_ROWS: usize = 5;
pub const UNDERNEATH: &str = "the file changed underneath";
pub const THEIRS: &str = "take theirs";
pub const MINE: &str = "keep mine";
pub const RELOAD: &str = "reload";
const TOLERANCE: f64 = 1e-9;
const GUTTER_PAD: f32 = 8.0;
const WAVE: f32 = 2.0;
const PROBLEM_GAP: f32 = 12.0;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Notice {
    Underneath,
    Refused(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Chord {
    Undo,
    Redo,
    Comment,
    Open,
}

pub struct Source {
    id: Id,
    file: PathBuf,
    text: String,
    base: String,
    seen: Option<u64>,
    editable: bool,
    cache: JobCache,
    edited: Option<f64>,
    started: Option<Range<usize>>,
    selection: Range<usize>,
    chars: Option<(usize, usize)>,
    place: bool,
    notice: Option<Notice>,
    table: Option<Table>,
    tables: TableCache,
    reported: Option<usize>,
    problems: Vec<Diagnostic>,
    spans: Vec<Option<Range<usize>>>,
    mark: Option<Range<usize>>,
    scroll_to: Option<usize>,
    jump: Option<usize>,
    rows: Vec<Rect>,
    underlines: Vec<Rect>,
    choices: Vec<Rect>,
    compare: Compare,
    head: Box<dyn Head>,
    opener: Box<dyn Opener>,
    open_key: Option<KeyboardShortcut>,
    toolbar: bool,
    bar: Vec<Rect>,
}

fn floor(text: &str, byte: usize) -> usize {
    let mut byte = byte.min(text.len());
    while !text.is_char_boundary(byte) {
        byte -= 1;
    }
    byte
}

fn char_index(text: &str, byte: usize) -> usize {
    text[..floor(text, byte)].chars().count()
}

fn byte_index(text: &str, char: usize) -> usize {
    text.char_indices()
        .nth(char)
        .map_or(text.len(), |(at, _)| at)
}

fn shifted(before: &str, after: &str, selection: Range<usize>) -> Range<usize> {
    let Some(splice) = TextSplice::between("", before, after) else {
        return floor(after, selection.start)..floor(after, selection.end);
    };
    let (start, end, new) = (splice.range.start, splice.range.end, splice.new.len());
    let shift = |byte: usize| {
        let moved = if byte <= start {
            byte
        } else if byte >= end {
            byte - end + start + new
        } else {
            start + (byte - start).min(new)
        };
        floor(after, moved)
    };
    shift(selection.start)..shift(selection.end)
}

fn chord(input: &mut InputState, open: Option<KeyboardShortcut>) -> Option<Chord> {
    if let Some(open) = open
        && input.consume_shortcut(&open)
    {
        return Some(Chord::Open);
    }
    let redo = KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z);
    if input.consume_shortcut(&redo) || input.consume_key(Modifiers::COMMAND, Key::Y) {
        Some(Chord::Redo)
    } else if input.consume_key(Modifiers::COMMAND, Key::Z) {
        Some(Chord::Undo)
    } else if input.consume_key(Modifiers::COMMAND, Key::Slash) {
        Some(Chord::Comment)
    } else {
        None
    }
}

fn wave(from: Pos2, to: f32, color: egui::Color32) -> Shape {
    let mut points = vec![from];
    let mut x = from.x;
    let mut up = true;
    while x < to {
        x = (x + WAVE).min(to);
        points.push(pos2(x, from.y + if up { -WAVE / 2.0 } else { 0.0 }));
        up = !up;
    }
    Shape::line(points, Stroke::new(1.0_f32, color))
}

struct Drawn<'a> {
    galley: &'a Arc<Galley>,
    at: Vec2,
}

impl Drawn<'_> {
    fn cursor(&self, text: &str, byte: usize) -> (usize, f32) {
        let cursor = CCursor::new(char_index(text, byte));
        let row = self.galley.layout_from_cursor(cursor).row;
        (row, self.galley.pos_from_cursor(cursor).min.x + self.at.x)
    }

    fn row(&self, row: usize) -> Rect {
        self.galley.rows[row].rect().translate(self.at)
    }

    fn spans(&self, text: &str, span: &Range<usize>) -> Vec<Rect> {
        let (first, left) = self.cursor(text, span.start);
        let (last, right) = self.cursor(text, span.end);
        (first..=last.min(self.galley.rows.len().saturating_sub(1)))
            .map(|row| {
                let rect = self.row(row);
                let x0 = if row == first { left } else { rect.min.x };
                let x1 = if row == last { right } else { rect.max.x };
                Rect::from_x_y_ranges(x0..=x1.max(x0), rect.y_range())
            })
            .collect()
    }
}

fn shown_path(file: &Path, own: &Path) -> String {
    let folder = own.parent().unwrap_or(Path::new(""));
    match file.strip_prefix(folder) {
        Ok(relative) if !folder.as_os_str().is_empty() => relative.display().to_string(),
        _ => file
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| file.display().to_string()),
    }
}

impl Source {
    pub fn new(id_salt: impl Hash, file: impl Into<PathBuf>) -> Source {
        Source {
            id: Id::new(id_salt),
            file: file.into(),
            text: String::new(),
            base: String::new(),
            seen: None,
            editable: true,
            cache: JobCache::default(),
            edited: None,
            started: None,
            selection: 0..0,
            chars: None,
            place: false,
            notice: None,
            table: None,
            tables: TableCache::default(),
            reported: None,
            problems: Vec::new(),
            spans: Vec::new(),
            mark: None,
            scroll_to: None,
            jump: None,
            rows: Vec::new(),
            underlines: Vec::new(),
            choices: Vec::new(),
            compare: Compare::default(),
            head: Box::new(Git),
            opener: Box::new(Process),
            open_key: None,
            toolbar: true,
            bar: Vec::new(),
        }
    }

    pub fn with_head(mut self, head: impl Head + 'static) -> Source {
        self.head = Box::new(head);
        self
    }

    pub fn with_opener(mut self, opener: impl Opener + 'static) -> Source {
        self.opener = Box::new(opener);
        self
    }

    pub fn open_shortcut(mut self, shortcut: Option<KeyboardShortcut>) -> Source {
        self.open_key = shortcut;
        self
    }

    pub fn toolbar(mut self, toolbar: bool) -> Source {
        self.toolbar = toolbar;
        self
    }

    pub fn editable(mut self, editable: bool) -> Source {
        self.editable = editable;
        self
    }

    pub fn set_editable(&mut self, editable: bool) {
        self.editable = editable;
    }

    pub fn id(&self) -> Id {
        self.id.with("text")
    }

    pub fn file(&self) -> &Path {
        &self.file
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    pub fn unapplied(&self) -> bool {
        self.text != self.base
    }

    pub fn problems(&self) -> &[Diagnostic] {
        &self.problems
    }

    pub fn pending(&self) -> bool {
        self.edited.is_some()
    }

    pub fn notice(&self) -> Option<&Notice> {
        self.notice.as_ref()
    }

    pub fn cache(&self) -> &JobCache {
        &self.cache
    }

    pub fn mark(&self) -> Option<Range<usize>> {
        self.mark.clone()
    }

    pub fn selection(&self) -> Range<usize> {
        self.selection.clone()
    }

    pub fn caret(&self) -> usize {
        self.selection.end
    }

    pub fn choices(&self) -> &[Rect] {
        &self.choices
    }

    pub fn table(&self) -> Option<&Table> {
        self.table.as_ref()
    }

    pub fn bar(&self) -> &[Rect] {
        &self.bar
    }

    pub fn diff_on(&self) -> bool {
        self.compare.on
    }

    pub fn basis(&self) -> Basis {
        self.compare.basis
    }

    pub fn diffs(&self) -> usize {
        self.compare.computed()
    }

    pub fn reverts(&self) -> Vec<Rect> {
        self.compare.reverts.iter().map(|(_, rect)| *rect).collect()
    }

    pub fn head_note(&self) -> Option<String> {
        self.compare.head_note()
    }

    pub fn set_diff(&mut self, on: bool) {
        self.compare.on = on;
        if on && !self.compare.probed() {
            self.refresh_head();
        }
    }

    pub fn set_basis(&mut self, basis: Basis) {
        self.compare.basis = basis;
        if basis == Basis::Head && !self.compare.probed() {
            self.refresh_head();
        }
    }

    pub fn refresh_head(&mut self) {
        self.compare.probe(self.head.as_ref(), &self.file);
    }

    pub fn diff(&mut self) -> Option<&Diff> {
        self.compare.refresh(&self.text)
    }

    pub fn editor_command(&self) -> Option<String> {
        self.opener.command()
    }

    pub fn open_note(&self) -> Option<&'static str> {
        self.opener.command().is_none().then_some(NO_EDITOR)
    }

    pub fn open_in_editor(&mut self) -> bool {
        let Some(command) = self.opener.command() else {
            self.refused(NO_EDITOR);
            return false;
        };
        match self.opener.open(&command, &self.file) {
            Ok(()) => true,
            Err(error) => {
                self.refused(format!("cannot start {command}: {error}"));
                false
            }
        }
    }

    pub fn select(&mut self, selection: Range<usize>) {
        self.selection = floor(&self.text, selection.start)..floor(&self.text, selection.end);
        self.place = true;
    }

    fn set_text(&mut self, text: String) {
        self.text = text;
        self.chars = None;
        self.selection =
            floor(&self.text, self.selection.start)..floor(&self.text, self.selection.end);
    }

    fn respan(&mut self) {
        self.spans = self
            .problems
            .iter()
            .map(|problem| (problem.file == self.file).then(|| problem.span(&self.text)))
            .collect();
    }

    fn recheck(&mut self, file: &File, check: &dyn Check) {
        self.problems = match (file.doc(), file.error()) {
            (Some(doc), _) => check.check(&self.file, file.text(), doc),
            (None, Some(error)) => vec![Diagnostic::parse(&self.file, file.text(), error)],
            (None, None) => Vec::new(),
        };
        self.respan();
    }

    fn adopt(&mut self, file: &File, check: &dyn Check) {
        if self.text != file.text() {
            let selection = shifted(&self.text, file.text(), self.selection.clone());
            self.set_text(file.text().to_string());
            self.selection = selection;
            self.place = true;
        }
        self.base.clone_from(&self.text);
        self.seen = Some(file.generation());
        self.edited = None;
        self.started = None;
        if self.notice == Some(Notice::Underneath) {
            self.notice = None;
        }
        self.recheck(file, check);
    }

    pub fn sync(&mut self, files: &Files, check: &dyn Check) -> bool {
        let Some(file) = files.get(&self.file) else {
            return false;
        };
        if self.seen != Some(file.generation()) || self.compare.on {
            self.compare.note_saved(file, &self.file);
        }
        if self.seen == Some(file.generation()) {
            return false;
        }
        if self.seen.is_some() && self.unapplied() && self.text != file.text() {
            self.notice = Some(Notice::Underneath);
            return false;
        }
        self.adopt(file, check);
        true
    }

    pub fn take_theirs(&mut self, files: &Files, check: &dyn Check) {
        if let Some(file) = files.get(&self.file) {
            self.adopt(file, check);
        }
        self.notice = None;
    }

    pub fn keep_mine(&mut self, files: &Files, now: f64) {
        if let Some(file) = files.get(&self.file) {
            self.base = file.text().to_string();
            self.seen = Some(file.generation());
            if self.unapplied() {
                self.edited = Some(now);
            }
        }
        self.notice = None;
    }

    pub fn edit(&mut self, text: impl Into<String>, now: f64) {
        let text = text.into();
        if text != self.text {
            self.started.get_or_insert(self.selection.clone());
            self.set_text(text);
            self.edited = Some(now);
        }
    }

    pub fn tick(
        &mut self,
        now: f64,
        files: &mut Files,
        history: &mut History,
        check: &dyn Check,
    ) -> Option<Changed> {
        let at = self.edited?;
        if now - at + TOLERANCE < DEBOUNCE {
            return None;
        }
        self.edited = None;
        self.commit(now, files, history, check)
    }

    fn refused(&mut self, error: impl ToString) {
        self.notice = Some(Notice::Refused(error.to_string()));
    }

    fn commit(
        &mut self,
        now: f64,
        files: &mut Files,
        history: &mut History,
        check: &dyn Check,
    ) -> Option<Changed> {
        let (problems, clean) = match diag::parse(&self.file, &self.text) {
            Err(error) => (vec![*error], false),
            Ok(doc) => {
                let problems = check.check(&self.file, &self.text, &doc);
                let clean = problems
                    .iter()
                    .all(|problem| problem.severity != Severity::Error);
                (problems, clean)
            }
        };
        self.problems = problems;
        self.respan();
        if !clean || !self.unapplied() || self.notice == Some(Notice::Underneath) {
            return None;
        }
        if files.generation_of(&self.file) != self.seen {
            self.notice = Some(Notice::Underneath);
            return None;
        }
        let before = self.started.clone().unwrap_or(self.selection.clone());
        let splice = TextSplice::between(&self.file, &self.base, &self.text)?
            .with_carets(before, self.selection.clone());
        match history.typed(files, splice, now, GAP) {
            Ok(changed) => {
                self.base.clone_from(&self.text);
                self.seen = files.generation_of(&self.file);
                self.started = None;
                self.notice = None;
                Some(changed)
            }
            Err(error) => {
                self.refused(error);
                None
            }
        }
    }

    fn follow(&mut self, files: &Files, check: &dyn Check, changed: &Changed) {
        if let Some(file) = files.get(&self.file) {
            self.adopt(file, check);
        }
        if let Some(caret) = &changed.caret
            && caret.file == self.file
        {
            self.select(caret.selection.clone());
        }
        if matches!(self.notice, Some(Notice::Refused(_))) {
            self.notice = None;
        }
    }

    fn revert(&mut self, files: &Files, check: &dyn Check) {
        let back = self.started.take();
        if let Some(file) = files.get(&self.file) {
            self.adopt(file, check);
        }
        if let Some(back) = back {
            self.select(back);
        }
    }

    pub fn undo(
        &mut self,
        now: f64,
        files: &mut Files,
        history: &mut History,
        check: &dyn Check,
    ) -> Option<Changed> {
        self.step(true, now, files, history, check)
    }

    pub fn redo(
        &mut self,
        now: f64,
        files: &mut Files,
        history: &mut History,
        check: &dyn Check,
    ) -> Option<Changed> {
        self.step(false, now, files, history, check)
    }

    fn step(
        &mut self,
        undo: bool,
        now: f64,
        files: &mut Files,
        history: &mut History,
        check: &dyn Check,
    ) -> Option<Changed> {
        if self.notice == Some(Notice::Underneath) {
            return None;
        }
        self.edited = None;
        if self.unapplied() && self.commit(now, files, history, check).is_none() {
            if undo && self.notice != Some(Notice::Underneath) {
                self.revert(files, check);
            }
            return None;
        }
        let stepped = if undo {
            history.undo(files)
        } else {
            history.redo(files)
        };
        match stepped {
            Ok(Some(changed)) => {
                self.follow(files, check, &changed);
                Some(changed)
            }
            Ok(None) => None,
            Err(error) => {
                self.refused(error);
                None
            }
        }
    }

    pub fn toggle_comment(
        &mut self,
        now: f64,
        files: &mut Files,
        history: &mut History,
        check: &dyn Check,
    ) -> Option<Changed> {
        if self.notice == Some(Notice::Underneath) {
            return None;
        }
        self.edited = None;
        if self.unapplied() {
            self.commit(now, files, history, check);
            if self.notice == Some(Notice::Underneath) {
                return None;
            }
        }
        let splice = toggle_comment(&self.file, &self.text, self.selection.clone())?;
        if self.unapplied() {
            let mut text = self.text.clone();
            text.replace_range(splice.range.clone(), &splice.new);
            self.started.get_or_insert(self.selection.clone());
            self.set_text(text);
            self.select(splice.caret_after);
            self.edited = Some(now);
            return None;
        }
        match history.apply(files, splice) {
            Ok(changed) => {
                self.follow(files, check, &changed);
                Some(changed)
            }
            Err(error) => {
                self.refused(error);
                None
            }
        }
    }

    pub fn revert_hunk(
        &mut self,
        index: usize,
        now: f64,
        files: &mut Files,
        history: &mut History,
        check: &dyn Check,
    ) -> Option<Changed> {
        if self.notice == Some(Notice::Underneath) {
            return None;
        }
        self.edited = None;
        if self.unapplied() {
            self.commit(now, files, history, check);
            if self.notice == Some(Notice::Underneath) {
                return None;
            }
        }
        self.compare.refresh(&self.text)?;
        let hunk = self.compare.hunk(index)?;
        let base = self.compare.base_of()?;
        let splice = hunk.revert(&self.file, base, &self.text);
        if self.unapplied() {
            let mut text = self.text.clone();
            text.replace_range(splice.range.clone(), &splice.new);
            self.started.get_or_insert(self.selection.clone());
            self.set_text(text);
            self.select(splice.caret_after);
            self.edited = Some(now);
            return None;
        }
        match history.apply(files, splice) {
            Ok(changed) => {
                self.follow(files, check, &changed);
                Some(changed)
            }
            Err(error) => {
                self.refused(error);
                None
            }
        }
    }

    pub fn reload(
        &mut self,
        files: &mut Files,
        history: &mut History,
        check: &dyn Check,
    ) -> Option<Changed> {
        let Ok(Disk::Outside(disk)) = files.disk(&self.file) else {
            return None;
        };
        let current = files.text(&self.file)?;
        let changed = match TextSplice::between(&self.file, current, &disk) {
            Some(splice) => match history.apply(files, splice.labelled(RELOAD)) {
                Ok(changed) => Some(changed),
                Err(error) => {
                    self.refused(error);
                    return None;
                }
            },
            None => None,
        };
        if let Err(error) = files.reload(&self.file, disk) {
            self.refused(error);
        }
        self.sync(files, check);
        changed
    }

    pub fn reveal(&mut self, range: Range<usize>) {
        let start = floor(&self.text, range.start);
        let end = floor(&self.text, range.end).max(start);
        self.mark = Some(start..end);
        self.scroll_to = Some(start);
    }

    pub fn clear_mark(&mut self) {
        self.mark = None;
    }

    pub fn jump(&mut self, byte: usize) {
        self.jump = Some(byte);
    }

    pub fn show(
        &mut self,
        ui: &mut Ui,
        files: &mut Files,
        history: &mut History,
        check: &dyn Check,
        select: &mut dyn FnMut(usize, &Table),
    ) -> Option<Changed> {
        let now = ui.input(|input| input.time);
        let theme = Theme::of(ui.ctx());
        self.sync(files, check);
        let mut changed = self.tick(now, files, history, check);
        let live = ui.memory(|memory| memory.has_focus(self.id()))
            || (self.compare.on && ui.ui_contains_pointer());
        if live {
            let open = self.open_key;
            let editable = self.editable;
            let done = match ui.input_mut(|input| chord(input, open)) {
                Some(Chord::Undo) if editable => self.undo(now, files, history, check),
                Some(Chord::Redo) if editable => self.redo(now, files, history, check),
                Some(Chord::Comment) if editable => self.toggle_comment(now, files, history, check),
                Some(Chord::Open) => {
                    self.open_in_editor();
                    None
                }
                _ => None,
            };
            changed = done.or(changed);
        }
        self.choices.clear();
        if self.toolbar {
            self.tools(ui, &theme);
        }
        if self.notice.is_some() {
            self.notice_row(ui, &theme, files, check, now);
        }
        let listed = self.problems.len().min(PROBLEM_ROWS);
        let list_height = if listed == 0 {
            0.0
        } else {
            listed as f32 * theme.row_height + theme.item_spacing
        };
        let text_height = (ui.available_height() - list_height).max(theme.row_height * 3.0);
        let background = theme.well;
        let before = self.selection.clone();
        let mut reverted = None;
        let typed = egui::Frame::new()
            .fill(background)
            .show(ui, |ui| {
                if self.compare.on {
                    reverted = self.compare.paint(ui, &theme, &self.text, text_height);
                    return false;
                }
                ScrollArea::both()
                    .id_salt(self.id.with("scroll"))
                    .auto_shrink([false, false])
                    .max_height(text_height)
                    .show(ui, |ui| self.editor(ui, &theme))
                    .inner
            })
            .inner;
        if let Some(hunk) = reverted {
            let done = self.revert_hunk(hunk, now, files, history, check);
            changed = done.or(changed);
            ui.ctx().request_repaint();
        }
        if typed {
            self.started.get_or_insert(before);
            self.edited = Some(now);
        }
        let focused = ui.memory(|memory| memory.has_focus(self.id()));
        self.report(focused, select);
        if let Some(at) = self.edited {
            ui.ctx()
                .request_repaint_after_secs((at + DEBOUNCE - now).max(0.0) as f32);
        }
        self.rows.clear();
        if listed > 0 {
            self.list(ui, list_height, &theme);
        }
        changed
    }

    fn tools(&mut self, ui: &mut Ui, theme: &Theme) {
        let mut rects = Vec::new();
        let (mut toggle, mut saved, mut head, mut open) = (false, false, false, false);
        let has_head = self.compare.has_head();
        let reason = self.compare.head_note();
        let note = self.open_note();
        ui.horizontal(|ui| {
            let button = ui
                .add(KeyCap::new("diff").button().lit(self.compare.on))
                .on_hover_text("show the buffer against the saved file or git HEAD");
            rects.push(button.rect);
            toggle = button.clicked();
            if self.compare.on {
                let button = ui.add(
                    KeyCap::new("saved")
                        .button()
                        .lit(self.compare.basis == Basis::Saved),
                );
                rects.push(button.rect);
                saved = button.clicked();
                let label = if self.compare.probed() && !has_head {
                    "no HEAD"
                } else {
                    "HEAD"
                };
                let button = ui
                    .add_enabled(
                        has_head,
                        KeyCap::new(label)
                            .button()
                            .lit(self.compare.basis == Basis::Head),
                    )
                    .on_disabled_hover_text(reason.unwrap_or_default());
                rects.push(button.rect);
                head = button.clicked();
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let button = ui
                    .add_enabled(note.is_none(), KeyCap::new("open in editor").button())
                    .on_hover_text("open the file in $VISUAL, else $EDITOR")
                    .on_disabled_hover_text(note.unwrap_or_default());
                rects.push(button.rect);
                open = button.clicked();
            });
        });
        ui.add_space(theme.item_spacing);
        self.bar = rects;
        if toggle {
            self.set_diff(!self.compare.on);
        }
        if saved {
            self.set_basis(Basis::Saved);
        }
        if head {
            self.set_basis(Basis::Head);
        }
        if open {
            self.open_in_editor();
        }
        if toggle || saved || head {
            ui.ctx().request_repaint();
        }
    }

    fn report(&mut self, focused: bool, select: &mut dyn FnMut(usize, &Table)) {
        let caret = self.caret();
        if self.reported == Some(caret) {
            return;
        }
        let fresh;
        let lexed = if self.cache.lexed().text() == self.text {
            self.cache.lexed()
        } else {
            fresh = Lexed::new(&self.text);
            &fresh
        };
        let table = self.tables.at(lexed, caret);
        let moved = match (&self.table, &table) {
            (Some(was), Some(now)) => !was.same(now),
            (None, None) => false,
            _ => true,
        };
        if moved
            && focused
            && self.reported.is_some()
            && let Some(table) = &table
        {
            select(caret, table);
        }
        self.reported = Some(caret);
        self.table = table;
    }

    fn notice_row(
        &mut self,
        ui: &mut Ui,
        theme: &Theme,
        files: &Files,
        check: &dyn Check,
        now: f64,
    ) {
        let Some(notice) = self.notice.clone() else {
            return;
        };
        let code = theme.font(theme.size_code);
        let (message, color, choose) = match &notice {
            Notice::Underneath => (UNDERNEATH, theme.warning, true),
            Notice::Refused(message) => (message.as_str(), theme.error, false),
        };
        let mut choice = None;
        let mut choices = Vec::new();
        ui.horizontal(|ui| {
            ui.add(Label::new(
                RichText::new(message).font(code.clone()).color(color),
            ));
            if choose {
                ui.add_space(PROBLEM_GAP);
                let theirs = ui.add(KeyCap::new(THEIRS).button());
                ui.add_space(theme.item_spacing);
                let mine = ui.add(KeyCap::new(MINE).button());
                choices = vec![theirs.rect, mine.rect];
                if theirs.clicked() {
                    choice = Some(true);
                } else if mine.clicked() {
                    choice = Some(false);
                }
            }
        });
        ui.add_space(theme.item_spacing);
        self.choices = choices;
        match choice {
            Some(true) => self.take_theirs(files, check),
            Some(false) => self.keep_mine(files, now),
            None => return,
        }
        ui.ctx().request_repaint();
    }

    fn editor(&mut self, ui: &mut Ui, theme: &Theme) -> bool {
        let code = theme.font(theme.size_code);
        let text_id = self.id();
        let ctx = ui.ctx().clone();
        if let Some(byte) = self.jump.take() {
            let byte = floor(&self.text, byte);
            self.selection = byte..byte;
            self.place = true;
            ctx.memory_mut(|memory| memory.request_focus(text_id));
            self.scroll_to = Some(byte);
        }
        if std::mem::take(&mut self.place) {
            let mut state = TextEditState::load(&ctx, text_id).unwrap_or_default();
            let anchor = CCursor::new(char_index(&self.text, self.selection.start));
            let cursor = CCursor::new(char_index(&self.text, self.selection.end));
            state
                .cursor
                .set_char_range(Some(CCursorRange::two(anchor, cursor)));
            state.store(&ctx, text_id);
            self.chars = None;
        }
        let lines = self.cache.lexed().lines().len().max(1);
        let digits = lines.to_string().len().max(2);
        let digit = ui.fonts_mut(|fonts| fonts.glyph_width(&code, '0'));
        let gutter = digits as f32 * digit + 2.0 * GUTTER_PAD;
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            let left = ui.cursor().min.x;
            ui.add_space(gutter);
            let behind = ui.painter().add(Shape::Noop);
            ui.visuals_mut().selection.bg_fill = theme.accent;
            let Source {
                text,
                cache,
                editable,
                ..
            } = self;
            let mut layouter = |ui: &Ui, buffer: &dyn TextBuffer, _: f32| {
                let pixels_per_point = ui.ctx().pixels_per_point();
                ui.fonts_mut(|fonts| {
                    cache.galley(fonts, pixels_per_point, buffer.as_str(), theme, &code)
                })
            };
            let mut view;
            let buffer: &mut dyn TextBuffer = if *editable {
                text
            } else {
                view = text.as_str();
                &mut view
            };
            let output = TextEdit::multiline(buffer)
                .id(text_id)
                .code_editor()
                .frame(false)
                .desired_width(f32::INFINITY)
                .desired_rows(1)
                .margin(Margin::symmetric(4, 0))
                .layouter(&mut layouter)
                .show(ui);
            let typed = *editable && output.response.changed();
            if *editable {
                let mut state = output.state.clone();
                state.clear_undoer();
                state.store(&ctx, text_id);
            }
            if typed {
                self.chars = None;
            }
            if let Some(range) = output.cursor_range {
                let chars = (range.secondary.index, range.primary.index);
                if self.chars != Some(chars) {
                    self.selection =
                        byte_index(&self.text, chars.0)..byte_index(&self.text, chars.1);
                    self.chars = Some(chars);
                }
            }
            let drawn = Drawn {
                galley: &output.galley,
                at: output.galley_pos.to_vec2(),
            };
            let clip = ui.clip_rect();
            if let Some(mark) = &self.mark {
                let rects = drawn.spans(&self.text, mark);
                if let (Some(first), Some(last)) = (rects.first(), rects.last()) {
                    let rect = Rect::from_x_y_ranges(
                        drawn.at.x..=clip.max.x.max(first.max.x),
                        first.min.y..=last.max.y,
                    );
                    ui.painter()
                        .set(behind, Shape::rect_filled(rect, 0.0, theme.accent_deep));
                }
            }
            self.underlines.clear();
            let hover = output.response.hover_pos();
            let mut hovered = Vec::new();
            for (problem, span) in self.problems.iter().zip(&self.spans) {
                let Some(span) = span else {
                    continue;
                };
                if span.end > self.text.len() || !self.text.is_char_boundary(span.end) {
                    continue;
                }
                let color = match problem.severity {
                    Severity::Error => theme.error,
                    Severity::Warning => theme.warning,
                };
                for rect in drawn.spans(&self.text, span) {
                    let y = rect.max.y - 1.0;
                    ui.painter().add(wave(
                        pos2(rect.min.x, y),
                        rect.max.x.max(rect.min.x + WAVE * 2.0),
                        color,
                    ));
                    if hover.is_some_and(|at| rect.expand(1.0).contains(at)) {
                        hovered.push(problem.message.clone());
                    }
                    self.underlines.push(rect);
                }
            }
            if !hovered.is_empty() {
                hovered.dedup();
                output.response.clone().on_hover_ui_at_pointer(|ui| {
                    for message in &hovered {
                        ui.label(message);
                    }
                });
            }
            let current = output
                .cursor_range
                .map(|range| output.galley.layout_from_cursor(range.primary).row);
            let rows = &output.galley.rows;
            let first = rows.partition_point(|row| row.rect().max.y + drawn.at.y < clip.min.y);
            for (index, _) in rows.iter().enumerate().skip(first) {
                let rect = drawn.row(index);
                if rect.min.y > clip.max.y {
                    break;
                }
                let color = if Some(index) == current {
                    theme.text2
                } else {
                    theme.dim
                };
                ui.painter().text(
                    pos2(left + gutter - GUTTER_PAD, rect.center().y),
                    Align2::RIGHT_CENTER,
                    (index + 1).to_string(),
                    code.clone(),
                    color,
                );
            }
            if let Some(byte) = self.scroll_to.take() {
                let (row, x) = drawn.cursor(&self.text, byte);
                let rect = drawn.row(row.min(rows.len().saturating_sub(1)));
                let target = Rect::from_x_y_ranges(left..=x.max(left), rect.y_range());
                ui.scroll_to_rect(target, Some(Align::Center));
            }
            typed
        })
        .inner
    }

    fn list(&mut self, ui: &mut Ui, height: f32, theme: &Theme) {
        let code = theme.font(theme.size_code);
        ui.add_space(theme.item_spacing);
        let mut clicked = None;
        ScrollArea::vertical()
            .id_salt(self.id.with("problems"))
            .max_height(height - theme.item_spacing)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                for (index, problem) in self.problems.iter().enumerate() {
                    let color = match problem.severity {
                        Severity::Error => theme.error,
                        Severity::Warning => theme.warning,
                    };
                    let mut job = LayoutJob::default();
                    job.append(
                        &format!(
                            "{}:{}:{}",
                            shown_path(&problem.file, &self.file),
                            problem.line,
                            problem.column
                        ),
                        0.0,
                        TextFormat::simple(code.clone(), color),
                    );
                    job.append(
                        &problem.message,
                        PROBLEM_GAP,
                        TextFormat::simple(code.clone(), theme.syntax.plain),
                    );
                    let response = ui.add(Label::new(job).sense(Sense::click()).truncate());
                    if response.clicked() {
                        clicked = Some(index);
                    }
                    self.rows.push(response.rect);
                }
            });
        if let Some(index) = clicked {
            let problem = &self.problems[index];
            if problem.file == self.file {
                self.jump = Some(problem.start(&self.text));
                ui.ctx().request_repaint();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_problem_names_its_file_from_the_panels_folder_never_the_machines_path() {
        let own = Path::new("/srv/work/scenes/stand.scene.toml");
        assert_eq!(
            shown_path(Path::new("/srv/work/scenes/stand.scene.toml"), own),
            "stand.scene.toml"
        );
        assert_eq!(
            shown_path(Path::new("/srv/work/scenes/lib/materials.toml"), own),
            "lib/materials.toml"
        );
        assert_eq!(
            shown_path(Path::new("/elsewhere/rig.toml"), own),
            "rig.toml"
        );
    }
    use crate::diag::NoCheck;
    use egui::Context;
    use pfx_editor_doc::TYPING;
    use pfx_editor_shell::context;
    use pfx_editor_shell::{Script, drive};
    use pfx_editor_style::fixture;
    use toml_edit::DocumentMut;

    const FILE: &str = "scene.toml";
    const GOOD: &str = "[object.lamp]\nname = \"lamp\"\nat = [0.0, 1.5, -2.0]\n";

    struct Open {
        files: Files,
        history: History,
        source: Source,
    }

    impl Open {
        fn new(text: &str, check: &dyn Check) -> Open {
            let mut files = Files::new();
            files.open(FILE, text);
            let mut source = Source::new("source", FILE);
            assert!(source.sync(&files, check));
            Open {
                files,
                history: History::default(),
                source,
            }
        }

        fn tick(&mut self, now: f64, check: &dyn Check) -> Option<Changed> {
            self.source
                .tick(now, &mut self.files, &mut self.history, check)
        }

        fn text(&self) -> &str {
            self.files.text(Path::new(FILE)).unwrap()
        }

        fn frame(&mut self, ctx: &Context, script: &Script) -> Vec<Option<Changed>> {
            let mut changed = Vec::new();
            let Open {
                files,
                history,
                source,
            } = self;
            drive(ctx, [480, 320], script, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    changed.push(source.show(ui, files, history, &check_lamps, &mut |_, _| {}));
                });
            });
            changed
        }
    }

    fn check_lamps(file: &Path, text: &str, doc: &DocumentMut) -> Vec<Diagnostic> {
        let Some(name) = doc
            .get("object")
            .and_then(|objects| objects.get("lamp"))
            .and_then(|lamp| lamp.get("name"))
        else {
            return Vec::new();
        };
        if name.as_str() == Some("lamp") {
            return Vec::new();
        }
        let at = text.find("name").unwrap();
        let mut found = Diagnostic::at(
            file,
            text,
            at..at + 4,
            Severity::Error,
            "bad-name",
            "a lamp is named lamp",
        );
        found.key = "object.lamp.name".into();
        vec![found]
    }

    #[test]
    fn an_edit_applies_only_after_the_debounce_with_no_other_edit() {
        let mut open = Open::new(GOOD, &NoCheck);
        open.source.edit(GOOD.replace("1.5", "2.5"), 1.0);
        assert!(open.source.pending());
        assert!(open.tick(1.10, &NoCheck).is_none());
        open.source.edit(GOOD.replace("1.5", "3.5"), 1.12);
        assert!(open.tick(1.20, &NoCheck).is_none());
        assert!(open.tick(1.26, &NoCheck).is_none());
        assert_eq!(open.text(), GOOD);
        let changed = open.tick(1.27, &NoCheck).unwrap();
        assert_eq!(changed.label, TYPING);
        assert_eq!(open.text(), GOOD.replace("1.5", "3.5"));
        assert!(!open.source.pending());
        assert!(!open.source.unapplied());
        assert_eq!(open.history.undo_len(), 1);
        assert!(open.tick(5.0, &NoCheck).is_none());
    }

    #[test]
    fn an_unchanged_edit_schedules_nothing() {
        let mut open = Open::new(GOOD, &NoCheck);
        open.source.edit(GOOD, 1.0);
        assert!(!open.source.pending());
    }

    #[test]
    fn a_broken_buffer_keeps_its_text_and_reports_the_parse_error_first() {
        let mut open = Open::new(GOOD, &check_lamps);
        let broken = GOOD.replace("name = \"lamp\"", "name = = \"lamp\"");
        open.source.edit(broken.clone(), 0.0);
        assert!(open.tick(0.2, &check_lamps).is_none());
        assert_eq!(open.source.text(), broken);
        assert_eq!(open.text(), GOOD);
        assert!(open.source.unapplied());
        let problems = open.source.problems();
        assert_eq!(problems.len(), 1);
        assert_eq!(problems[0].code, diag::PARSE);
        assert_eq!((problems[0].line, problems[0].column), (2, 8));
        open.source.edit(GOOD.replace("\"lamp\"", "\"bulb\""), 1.0);
        assert!(open.tick(1.2, &check_lamps).is_none());
        assert_eq!(open.source.problems()[0].code, "bad-name");
        assert_eq!(open.source.spans[0], Some(14..18));
        open.source.edit(GOOD, 2.0);
        assert!(open.tick(2.2, &check_lamps).is_none());
        assert!(open.source.problems().is_empty());
        assert!(!open.source.unapplied());
        assert_eq!(open.history.undo_len(), 0);
    }

    #[test]
    fn warnings_still_apply() {
        let warn = |file: &Path, text: &str, _: &DocumentMut| {
            vec![Diagnostic::at(
                file,
                text,
                0..1,
                Severity::Warning,
                "w",
                "a warning",
            )]
        };
        let mut open = Open::new(GOOD, &warn);
        assert_eq!(open.source.problems().len(), 1);
        open.source.edit(GOOD.replace("1.5", "2.5"), 0.0);
        assert!(open.tick(0.2, &warn).is_some());
        assert_eq!(open.source.problems().len(), 1);
    }

    #[test]
    fn diagnostics_of_other_files_are_listed_but_not_underlined() {
        let elsewhere = |_: &Path, text: &str, _: &DocumentMut| {
            vec![Diagnostic::at(
                "other.toml",
                text,
                0..1,
                Severity::Error,
                "e",
                "held twice",
            )]
        };
        let mut open = Open::new(GOOD, &elsewhere);
        open.source.edit(GOOD.replace("1.5", "2.5"), 0.0);
        assert!(open.tick(0.2, &elsewhere).is_none());
        assert_eq!(open.source.spans, vec![None]);
    }

    #[test]
    fn an_undo_over_unapplied_broken_text_puts_back_the_applied_text() {
        let mut open = Open::new(GOOD, &NoCheck);
        open.source.select(3..3);
        open.source.edit(GOOD.replace("[object", "[[object"), 0.0);
        let Open {
            files,
            history,
            source,
        } = &mut open;
        assert!(source.undo(0.05, files, history, &NoCheck).is_none());
        assert_eq!(source.text(), GOOD);
        assert_eq!(source.selection(), 3..3);
        assert!(!source.unapplied() && !source.pending());
        source.edit(GOOD.replace("1.5", "2.5"), 1.0);
        assert!(source.undo(1.05, files, history, &NoCheck).is_some());
        assert_eq!(source.text(), GOOD);
        assert_eq!(history.redo_len(), 1);
        assert!(source.redo(1.1, files, history, &NoCheck).is_some());
        assert_eq!(source.text(), GOOD.replace("1.5", "2.5"));
    }

    #[test]
    fn a_selection_follows_an_outside_splice() {
        let text = "abcdef";
        assert_eq!(shifted(text, "abXXcdef", 3..5), 5..7);
        assert_eq!(shifted(text, "abXXcdef", 0..2), 0..2);
        assert_eq!(shifted(text, "aZf", 1..5), 1..2);
        assert_eq!(shifted(text, text, 4..9), 4..6);
        assert_eq!(shifted("é = 1", "é", 0..5), 0..2);
    }

    #[test]
    fn the_panel_underlines_lists_and_jumps_to_a_problem() {
        let ctx = context(&fixture::theme());
        let broken = format!("{GOOD}\n[object.box]\nsize = [1, 2\n");
        let mut open = Open::new(&broken, &check_lamps);
        open.frame(&ctx, &Script::default());
        assert_eq!(open.source.rows.len(), 1);
        assert!(!open.source.underlines.is_empty());
        let row = open.source.rows[0].center();
        let script = Script::parse(&format!("click {} {}\nframe\nframe", row.x, row.y)).unwrap();
        open.frame(&ctx, &script);
        let start = open.source.problems()[0].start(&broken);
        assert_eq!(open.source.caret(), start);
        assert_eq!(open.source.text(), broken);
    }

    #[test]
    fn typing_into_the_panel_applies_after_the_debounce() {
        let ctx = context(&fixture::theme());
        let mut open = Open::new(GOOD, &check_lamps);
        open.source.jump(GOOD.find("lamp\"").unwrap());
        open.frame(&ctx, &Script::default());
        let mut steps = String::from("text x\n");
        for _ in 0..12 {
            steps.push_str("frame\n");
        }
        let changed = open.frame(&ctx, &Script::parse(&steps).unwrap());
        assert_eq!(open.source.text(), GOOD.replace("\"lamp\"", "\"xlamp\""));
        assert_eq!(changed.iter().filter(|one| one.is_some()).count(), 0);
        assert_eq!(open.source.problems()[0].code, "bad-name");
        assert_eq!(open.text(), GOOD);
        assert!(!open.source.pending());
        assert!(open.source.unapplied());
    }

    #[test]
    fn a_read_only_panel_keeps_its_text() {
        let ctx = context(&fixture::theme());
        let mut open = Open::new(GOOD, &NoCheck);
        open.source.set_editable(false);
        open.source.jump(3);
        open.frame(&ctx, &Script::default());
        open.frame(
            &ctx,
            &Script::parse("text typed\nkey ctrl+/\nframe").unwrap(),
        );
        assert_eq!(open.source.text(), GOOD);
        assert_eq!(open.text(), GOOD);
        assert!(!open.source.pending());
    }

    #[test]
    fn a_frame_with_no_change_builds_no_job() {
        let ctx = context(&fixture::theme());
        let mut open = Open::new(GOOD, &NoCheck);
        open.frame(&ctx, &Script::default());
        let misses = open.source.cache().misses();
        open.frame(&ctx, &Script::parse("frame\nframe\nframe").unwrap());
        assert_eq!(open.source.cache().misses(), misses);
        assert!(open.source.cache().hits() >= 3);
    }

    #[test]
    fn reveal_marks_a_span() {
        let mut open = Open::new(GOOD, &NoCheck);
        assert_eq!(open.source.text(), GOOD);
        open.source.reveal(14..GOOD.len() + 10);
        assert_eq!(open.source.mark(), Some(14..GOOD.len()));
        let ctx = context(&fixture::theme());
        open.frame(&ctx, &Script::default());
        open.source.clear_mark();
        assert_eq!(open.source.mark(), None);
    }
}
