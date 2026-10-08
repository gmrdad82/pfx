use egui::{Align2, CornerRadius, Response, Sense, Stroke, StrokeKind, Ui, Vec2, Widget};

use super::State;
use crate::theme::Theme;

pub struct KeyCap<'a> {
    key: &'a str,
    lit: bool,
    button: bool,
}

impl<'a> KeyCap<'a> {
    pub fn new(key: &'a str) -> KeyCap<'a> {
        KeyCap {
            key,
            lit: false,
            button: false,
        }
    }

    pub fn lit(mut self, lit: bool) -> KeyCap<'a> {
        self.lit = lit;
        self
    }

    pub fn button(mut self) -> KeyCap<'a> {
        self.button = true;
        self
    }
}

impl Widget for KeyCap<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let theme = Theme::of(ui.ctx());
        let key_font = theme.font(theme.size_code);
        let width = ui
            .painter()
            .layout_no_wrap(self.key.into(), key_font.clone(), theme.text)
            .size()
            .x;
        let size = Vec2::new((width + 10.0).round().max(18.0), 17.0);
        let sense = if self.button {
            Sense::click()
        } else {
            Sense::hover()
        };
        let (rect, response) = ui.allocate_exact_size(size, sense);
        if !ui.is_rect_visible(rect) {
            return response;
        }
        let state = if self.button {
            State::of(&response, self.lit)
        } else if self.lit {
            State::Active
        } else {
            State::Rest
        };
        let (fill, border, text) = match state {
            State::Rest if !ui.is_enabled() => (theme.surface, theme.line, theme.dim),
            State::Rest => (theme.surface, theme.line, theme.text),
            State::Hovered => (theme.accent_deep, theme.accent, theme.text),
            State::Active => (theme.accent_bright, theme.accent_bright, theme.on_accent),
        };
        let painter = ui.painter();
        painter.rect(
            rect,
            CornerRadius::same(theme.radius),
            fill,
            Stroke::new(theme.stroke, border),
            StrokeKind::Inside,
        );
        painter.hline(
            rect.x_range().shrink(1.0),
            rect.bottom() - 1.5,
            Stroke::new(
                theme.stroke,
                if state == State::Active {
                    theme.accent_bright
                } else {
                    border
                },
            ),
        );
        painter.text(
            rect.center() - Vec2::new(0.0, 0.5),
            Align2::CENTER_CENTER,
            self.key,
            key_font,
            text,
        );
        response
    }
}
