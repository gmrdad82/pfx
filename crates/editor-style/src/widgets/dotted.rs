use egui::{Pos2, Response, Sense, Shape, Stroke, Ui, Vec2, Widget};

use crate::theme::Theme;

#[derive(Default)]
pub struct Dotted {
    active: bool,
}

impl Dotted {
    pub fn new() -> Dotted {
        Dotted::default()
    }

    pub fn active(mut self, active: bool) -> Dotted {
        self.active = active;
        self
    }
}

impl Widget for Dotted {
    fn ui(self, ui: &mut Ui) -> Response {
        let size = Vec2::new(ui.available_width(), 7.0);
        let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
        if !ui.is_rect_visible(rect) {
            return response;
        }
        let theme = Theme::of(ui.ctx());
        let colour = if self.active {
            theme.accent_bright
        } else {
            theme.track
        };
        let y = rect.center().y.floor() + 0.5;
        ui.painter().extend(Shape::dashed_line(
            &[Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)],
            Stroke::new(theme.stroke, colour),
            theme.dash,
            theme.dash,
        ));
        response
    }
}
