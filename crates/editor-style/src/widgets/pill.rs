use egui::{Align2, Color32, CornerRadius, Response, Sense, Stroke, StrokeKind, Ui, Vec2, Widget};

use super::{place, shaped};
use crate::theme::Theme;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Neutral,
    Ok,
    Busy,
    Danger,
}

impl Tone {
    fn colours(self, theme: &Theme) -> (Color32, Color32) {
        match self {
            Tone::Neutral => (theme.surface, theme.text2),
            Tone::Ok => (theme.ok_tint, theme.ok),
            Tone::Busy => (theme.hot_dim, theme.hot),
            Tone::Danger => (theme.error_tint, theme.error),
        }
    }
}

pub struct Pill<'a> {
    text: &'a str,
    tone: Tone,
    active: bool,
}

impl<'a> Pill<'a> {
    pub fn new(text: &'a str, tone: Tone) -> Pill<'a> {
        Pill {
            text,
            tone,
            active: false,
        }
    }

    pub fn active(mut self, active: bool) -> Pill<'a> {
        self.active = active;
        self
    }
}

impl Widget for Pill<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let theme = Theme::of(ui.ctx());
        let (fill, ink) = self.tone.colours(&theme);
        let galley = shaped(
            ui.painter(),
            self.text,
            theme.font(theme.size_code),
            ink,
            theme.labels,
        );
        let width = galley.size().x;
        let size = Vec2::new((width + 22.0).round(), 16.0);
        let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
        if !ui.is_rect_visible(rect) {
            return response;
        }
        let border = if self.active { ink } else { fill };
        let painter = ui.painter();
        painter.rect(
            rect,
            CornerRadius::same(theme.radius),
            fill,
            Stroke::new(theme.stroke, border),
            StrokeKind::Inside,
        );
        painter.circle_filled(rect.left_center() + Vec2::new(8.0, 0.0), 2.5, ink);
        place(
            painter,
            rect.left_center() + Vec2::new(15.0, -0.5),
            Align2::LEFT_CENTER,
            galley,
            ink,
        );
        response
    }
}
