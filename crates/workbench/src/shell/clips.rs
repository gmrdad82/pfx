use super::*;
use crate::clipboard;
use pfx_play::Clipboard;

const SCENE_CLIP: &str = "copy, cut and paste work on recipe objects; in a scene, ctrl d duplicates and del deletes the selection";

impl Shell {
    fn chosen_names(&self) -> Vec<String> {
        self.object
            .and_then(|object| self.document.objects.get(object))
            .map(|object| vec![object.name.clone()])
            .unwrap_or_default()
    }

    fn loaded_recipe(&self) -> Option<&pfx_recipe::Loaded> {
        self.editor.as_ref().and_then(Editor::loaded)
    }

    pub(super) fn copy_objects(&mut self, ctx: &Context, cut: bool) {
        if self.edits_scene() {
            self.said_file(SCENE_CLIP.to_string());
            return;
        }
        let names = self.chosen_names();
        let Some(loaded) = self.loaded_recipe() else {
            return;
        };
        let copied = clipboard::copy(loaded, &names);
        let removed = if cut {
            clipboard::remove(loaded, &names).map(Some)
        } else {
            Ok(None)
        };
        match (copied, removed) {
            (Ok(text), Ok(removed)) => {
                self.board.set_text(&text);
                ctx.copy_text(text);
                let word = if cut { "cut" } else { "copied" };
                match removed {
                    Some((patches, names)) => {
                        self.edits
                            .push(format!("cut {}", names.join(", ")), patches);
                        self.object = None;
                    }
                    None => self.said_file(format!("{word} {}", names.join(", "))),
                }
            }
            (Err(error), _) | (_, Err(error)) => self.fail(error),
        }
    }

    pub(super) fn paste_objects(&mut self, text: Option<String>) {
        if self.edits_scene() {
            self.said_file(SCENE_CLIP.to_string());
            return;
        }
        let Some(text) = text
            .filter(|text| text.trim_start().starts_with(clipboard::HEADER))
            .or_else(|| self.board.get_text())
        else {
            self.said_file("nothing to paste: copy an object first".to_string());
            return;
        };
        self.paste_text(&text, "paste");
    }

    fn paste_text(&mut self, text: &str, verb: &str) {
        let Some(loaded) = self.loaded_recipe() else {
            return;
        };
        match clipboard::paste(loaded, text) {
            Ok(pasted) => {
                let label = format!("{verb} {}", pasted.names.join(", "));
                if !pasted.kept.is_empty() {
                    self.said_file(format!(
                        "{label}: kept this document's own {}",
                        pasted.kept.join(", ")
                    ));
                }
                self.edits.push(label, pasted.patches);
            }
            Err(error) => self.fail(error),
        }
    }

    pub(super) fn duplicate_objects(&mut self) {
        if let Some(session) = self.edit.as_deref_mut().filter(|_| !self.playing_recipe) {
            session.editor.duplicate();
            return;
        }
        let names = self.chosen_names();
        let Some(loaded) = self.loaded_recipe() else {
            return;
        };
        match clipboard::copy(loaded, &names) {
            Ok(text) => self.paste_text(&text, "duplicate"),
            Err(error) => self.fail(error),
        }
    }

    pub(super) fn delete_objects(&mut self) {
        if let Some(session) = self.edit.as_deref_mut().filter(|_| !self.playing_recipe) {
            session.editor.delete();
            return;
        }
        let names = self.chosen_names();
        let Some(loaded) = self.loaded_recipe() else {
            return;
        };
        match clipboard::remove(loaded, &names) {
            Ok((patches, names)) => {
                self.edits
                    .push(format!("delete {}", names.join(", ")), patches);
                self.object = None;
            }
            Err(error) => self.fail(error),
        }
    }

    pub fn history(&self) -> (Vec<String>, Vec<String>) {
        let own = |labels: (Vec<&str>, Vec<&str>)| {
            (
                labels.0.into_iter().map(str::to_string).collect(),
                labels.1.into_iter().map(str::to_string).collect(),
            )
        };
        match (self.session().filter(|_| self.edits_scene()), &self.editor) {
            (Some(session), _) => own((
                session.editor.history.undo_labels(),
                session.editor.history.redo_labels(),
            )),
            (None, Some(editor)) => own(editor.labels()),
            (None, None) => (Vec::new(), Vec::new()),
        }
    }

    pub(super) fn edit_menu(&mut self, ui: &mut Ui) {
        let palette = Palette::of(ui.ctx());
        let (undo, redo) = self.history();
        let recipe = self.editor.is_some() && !self.edits_scene();
        let scene = self.edits_scene();
        let chosen = recipe && self.object.is_some()
            || scene
                && self
                    .session()
                    .is_some_and(|session| session.editor.selected_object().is_some());
        let mut picked: Vec<Command> = Vec::new();
        let title = RichText::new("Edit")
            .font(Theme::of(ui.ctx()).font(SIZE_BODY))
            .color(palette.text_primary);
        ui.menu_button(title, |ui| {
            let last = |list: &[String]| list.last().cloned().unwrap_or_default();
            for (label, keys, enabled, command) in [
                (
                    format!("Undo {}", last(&undo)),
                    "Ctrl+Z",
                    !undo.is_empty(),
                    Command::Undo,
                ),
                (
                    format!("Redo {}", last(&redo)),
                    "Ctrl+Shift+Z",
                    !redo.is_empty(),
                    Command::Redo,
                ),
                ("Cut".to_string(), "Ctrl+X", recipe && chosen, Command::Cut),
                (
                    "Copy".to_string(),
                    "Ctrl+C",
                    recipe && chosen,
                    Command::Copy,
                ),
                ("Paste".to_string(), "Ctrl+V", recipe, Command::Paste),
                (
                    "Duplicate".to_string(),
                    "Ctrl+D",
                    chosen,
                    Command::Duplicate,
                ),
                ("Delete".to_string(), "Del", chosen, Command::Delete),
            ] {
                if ui
                    .add_enabled(enabled, Button::new(label).shortcut_text(keys))
                    .clicked()
                {
                    picked.push(command);
                    ui.close();
                }
            }
            ui.separator();
            ui.label(small(ui, "HISTORY", palette.text_muted));
            if undo.is_empty() && redo.is_empty() {
                ui.label(small(ui, "no edits yet", palette.text_dim));
            }
            for (depth, label) in redo.iter().enumerate() {
                let steps = redo.len() - depth;
                let text = RichText::new(format!("↷ {label}"))
                    .font(Theme::of(ui.ctx()).font(SIZE_SMALL))
                    .color(palette.text_dim);
                if ui.add(Button::new(text).frame(false)).clicked() {
                    picked.extend(std::iter::repeat_n(Command::Redo, steps));
                    ui.close();
                }
            }
            for (depth, label) in undo.iter().rev().enumerate() {
                let colour = if depth == 0 {
                    palette.accent
                } else {
                    palette.text_secondary
                };
                let text = RichText::new(format!("● {label}"))
                    .font(Theme::of(ui.ctx()).font(SIZE_SMALL))
                    .color(colour);
                let response = ui.add(Button::new(text).frame(false));
                if response.clicked() && depth > 0 {
                    picked.extend(std::iter::repeat_n(Command::Undo, depth));
                    ui.close();
                }
            }
        });
        let ctx = ui.ctx().clone();
        for command in picked {
            if command == Command::Paste {
                self.pasted = None;
            }
            self.command(&ctx, command);
            if matches!(command, Command::Undo | Command::Redo) {
                self.drain(&ctx);
            }
        }
    }
}
