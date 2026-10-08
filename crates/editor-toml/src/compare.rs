use std::path::Path;

use egui::{Align, Align2, Layout, Rect, ScrollArea, Sense, Shape, Ui, UiBuilder, pos2, vec2};
use pfx_editor_doc::File;
use pfx_editor_doc::Stamp;
use pfx_editor_style::Theme;
use pfx_editor_style::widgets::KeyCap;

use crate::diff::{Diff, Hunk, Mark, REVERT, Row};
use crate::host::{Head, NoHead};

pub const NO_SAVED: &str = "no saved text to compare with";
const GUTTER_PAD: f32 = 8.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Basis {
    #[default]
    Saved,
    Head,
}

struct Cached {
    basis: Basis,
    base: String,
    text: String,
    diff: Diff,
}

#[derive(Default)]
pub struct Compare {
    pub(crate) on: bool,
    pub(crate) basis: Basis,
    saved: Option<String>,
    head: Option<Result<String, NoHead>>,
    cached: Option<Cached>,
    computed: usize,
    pub(crate) reverts: Vec<(usize, Rect)>,
}

impl Compare {
    pub fn note_saved(&mut self, file: &File, path: &Path) {
        if !file.dirty() {
            self.saved = Some(file.text().to_string());
        } else if self.saved.is_none()
            && let Ok(disk) = std::fs::read_to_string(path)
            && file.disk_stamp() == Some(Stamp::of(&disk))
        {
            self.saved = Some(disk);
        }
    }

    pub fn probe(&mut self, head: &dyn Head, path: &Path) {
        self.head = Some(head.show(path));
    }

    pub fn probed(&self) -> bool {
        self.head.is_some()
    }

    pub fn head_note(&self) -> Option<String> {
        match &self.head {
            Some(Err(why)) => Some(why.to_string()),
            _ => None,
        }
    }

    pub fn has_head(&self) -> bool {
        matches!(self.head, Some(Ok(_)))
    }

    pub fn base(&self) -> Result<&str, String> {
        match self.basis {
            Basis::Saved => self.saved.as_deref().ok_or_else(|| NO_SAVED.to_string()),
            Basis::Head => match &self.head {
                Some(Ok(text)) => Ok(text),
                Some(Err(why)) => Err(why.to_string()),
                None => Err(NoHead::Failed("not read yet".into()).to_string()),
            },
        }
    }

    pub fn computed(&self) -> usize {
        self.computed
    }

    pub fn refresh(&mut self, text: &str) -> Option<&Diff> {
        let (base, basis) = match (self.base(), self.basis) {
            (Ok(base), basis) => (base.to_string(), basis),
            (Err(_), _) => return None,
        };
        let stale = !matches!(&self.cached, Some(cached)
            if cached.basis == basis && cached.base == base && cached.text == text);
        if stale {
            self.computed += 1;
            self.cached = Some(Cached {
                basis,
                diff: Diff::between(&base, text),
                base,
                text: text.to_string(),
            });
        }
        self.cached.as_ref().map(|cached| &cached.diff)
    }

    pub fn hunk(&self, index: usize) -> Option<Hunk> {
        self.cached.as_ref()?.diff.hunks().get(index).cloned()
    }

    pub fn base_of(&self) -> Option<&str> {
        self.cached.as_ref().map(|cached| cached.base.as_str())
    }

    pub fn paint(&mut self, ui: &mut Ui, theme: &Theme, text: &str, height: f32) -> Option<usize> {
        self.reverts.clear();
        let code = theme.font(theme.size_code);
        if self.refresh(text).is_none() {
            let why = self.base().err().unwrap_or_default();
            ui.add_space(theme.item_spacing);
            ui.label(egui::RichText::new(why).font(code).color(theme.warning));
            return None;
        }
        let cached = self.cached.as_ref()?;
        let diff = &cached.diff;
        let name = match self.basis {
            Basis::Saved => "the saved file",
            Basis::Head => "git HEAD",
        };
        if diff.is_empty() {
            ui.add_space(theme.item_spacing);
            ui.label(
                egui::RichText::new(format!("no changes against {name}"))
                    .font(code)
                    .color(theme.dim),
            );
            return None;
        }
        let rows = diff.rows();
        let row_height = theme.row_height;
        let digits = (text.lines().count().max(1)).to_string().len().max(2);
        let digit = ui.fonts_mut(|fonts| fonts.glyph_width(&code, '0'));
        let gutter = digits as f32 * digit + 2.0 * GUTTER_PAD;
        let mut reverts = Vec::new();
        ui.spacing_mut().item_spacing.y = 0.0;
        ScrollArea::vertical()
            .id_salt("pfx-editor-toml-diff")
            .auto_shrink([false, false])
            .max_height(height)
            .show_rows(ui, row_height, rows.len(), |ui, range| {
                for index in range {
                    let (rect, _) = ui.allocate_exact_size(
                        vec2(ui.available_width(), row_height),
                        Sense::hover(),
                    );
                    let mid = rect.center().y;
                    let text_at = pos2(rect.min.x + gutter + GUTTER_PAD, mid);
                    let marked = |hunk: usize| match diff.hunks()[hunk].mark {
                        Mark::Add => theme.diff.add,
                        Mark::Del => theme.diff.del,
                        Mark::Change => theme.diff.change,
                    };
                    let fill = |color| {
                        Shape::rect_filled(
                            Rect::from_min_max(pos2(rect.min.x + gutter, rect.min.y), rect.max),
                            0.0,
                            color,
                        )
                    };
                    let number = |ui: &Ui, line: usize| {
                        ui.painter().text(
                            pos2(rect.min.x + gutter - GUTTER_PAD, mid),
                            Align2::RIGHT_CENTER,
                            (line + 1).to_string(),
                            code.clone(),
                            theme.dim,
                        );
                    };
                    match &rows[index] {
                        Row::Head(hunk) => {
                            let color = marked(*hunk);
                            ui.painter().add(fill(color));
                            let h = &diff.hunks()[*hunk];
                            let label =
                                format!("@@ -{} +{} @@", span(&h.old_lines), span(&h.new_lines));
                            ui.painter().text(
                                text_at,
                                Align2::LEFT_CENTER,
                                label,
                                code.clone(),
                                theme.muted,
                            );
                            let mut child = ui.new_child(
                                UiBuilder::new()
                                    .max_rect(rect.shrink2(vec2(GUTTER_PAD, 0.0)))
                                    .layout(Layout::right_to_left(Align::Center)),
                            );
                            let button = child
                                .add(KeyCap::new(REVERT).button())
                                .on_hover_text("put the base text back over this hunk");
                            if button.clicked() {
                                reverts.push(*hunk);
                            }
                            self.reverts.push((*hunk, button.rect));
                        }
                        Row::Context { line, text } => {
                            number(ui, *line);
                            ui.painter().text(
                                text_at,
                                Align2::LEFT_CENTER,
                                text,
                                code.clone(),
                                theme.text2,
                            );
                        }
                        Row::Fold(count) => {
                            ui.painter().text(
                                text_at,
                                Align2::LEFT_CENTER,
                                format!("{count} unchanged lines"),
                                code.clone(),
                                theme.dim,
                            );
                        }
                        Row::Del { text, .. } => {
                            ui.painter().add(fill(theme.diff.del));
                            ui.painter().text(
                                text_at,
                                Align2::LEFT_CENTER,
                                format!("- {text}"),
                                code.clone(),
                                theme.error,
                            );
                        }
                        Row::Add { line, text, .. } => {
                            number(ui, *line);
                            ui.painter().add(fill(theme.diff.add));
                            ui.painter().text(
                                text_at,
                                Align2::LEFT_CENTER,
                                format!("+ {text}"),
                                code.clone(),
                                theme.ok,
                            );
                        }
                    }
                }
            });
        reverts.first().copied()
    }
}

fn span(lines: &std::ops::Range<usize>) -> String {
    match lines.len() {
        0 => format!("{},0", lines.start),
        1 => (lines.start + 1).to_string(),
        count => format!("{},{}", lines.start + 1, count),
    }
}
