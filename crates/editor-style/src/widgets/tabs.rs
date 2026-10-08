use egui::{Align2, CornerRadius, Id, Rect, Response, Sense, Stroke, Ui, Vec2, Widget};

use super::State;
use crate::theme::Theme;

pub struct Tabs<'a> {
    selected: &'a mut Option<usize>,
    names: &'a [&'a str],
}

impl<'a> Tabs<'a> {
    pub fn new(selected: &'a mut Option<usize>, names: &'a [&'a str]) -> Tabs<'a> {
        Tabs { selected, names }
    }

    pub fn id(name: &str) -> Id {
        Id::new(("editor tab", name))
    }
}

impl Widget for Tabs<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let theme = Theme::of(ui.ctx());
        let text_font = theme.font(theme.size_body);
        let mut whole: Option<Response> = None;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for (index, name) in self.names.iter().enumerate() {
                let width = ui
                    .painter()
                    .layout_no_wrap((*name).into(), text_font.clone(), theme.text)
                    .size()
                    .x;
                let size = Vec2::new((width + 24.0).round(), 24.0);
                let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
                let mut response = ui.interact(rect, Tabs::id(name), Sense::click());
                if response.clicked() {
                    *self.selected = Some(index);
                    response.mark_changed();
                }
                let state = State::of(&response, *self.selected == Some(index));
                let (fill, text) = match state {
                    State::Rest => (theme.surface, theme.text2),
                    State::Hovered => (theme.accent_deep, theme.text),
                    State::Active => (theme.bg, theme.text),
                };
                let painter = ui.painter();
                painter.rect_filled(rect, CornerRadius::same(theme.radius), fill);
                painter.text(
                    rect.center() - Vec2::new(0.0, 1.0),
                    Align2::CENTER_CENTER,
                    name,
                    text_font.clone(),
                    text,
                );
                if state == State::Active {
                    let underline = Rect::from_min_max(
                        rect.left_bottom() - Vec2::new(0.0, 2.0),
                        rect.right_bottom(),
                    );
                    painter.rect_filled(underline, CornerRadius::ZERO, theme.accent_bright);
                } else {
                    painter.hline(
                        rect.x_range(),
                        rect.bottom() - 0.5,
                        Stroke::new(theme.stroke, theme.line),
                    );
                }
                whole = Some(match whole.take() {
                    Some(all) => all.union(response),
                    None => response,
                });
            }
        });
        whole.unwrap_or_else(|| ui.allocate_response(Vec2::ZERO, Sense::hover()))
    }
}
