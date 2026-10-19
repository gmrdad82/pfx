use egui::{
    Align2, Color32, CornerRadius, Id, Pos2, Rect, Response, Sense, Stroke, StrokeKind, TextureId,
    Ui, Vec2, Widget,
};

use super::{State, place, shaped};
use crate::theme::Theme;
use crate::tokens::{HEADER_CODE, HEADER_EXTRA, HEADER_GAP, MARKER_RADIUS};

pub struct Header<'a> {
    code: &'a str,
    title: &'a str,
    info: Option<&'a str>,
    focused: bool,
    width: Option<f32>,
    id: Option<Id>,
    mark: Option<(TextureId, Vec2)>,
}

impl<'a> Header<'a> {
    pub fn new(code: &'a str, title: &'a str) -> Header<'a> {
        Header {
            code,
            title,
            info: None,
            focused: false,
            width: None,
            id: None,
            mark: None,
        }
    }

    pub fn info(mut self, info: &'a str) -> Header<'a> {
        self.info = Some(info);
        self
    }

    pub fn width(mut self, width: f32) -> Header<'a> {
        self.width = Some(width);
        self
    }

    pub fn id(mut self, id: Id) -> Header<'a> {
        self.id = Some(id);
        self
    }

    pub fn mark(mut self, texture: TextureId, size: Vec2) -> Header<'a> {
        self.mark = Some((texture, size));
        self
    }

    pub fn focused(mut self, focused: bool) -> Header<'a> {
        self.focused = focused;
        self
    }
}

impl Widget for Header<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let theme = Theme::of(ui.ctx());
        let size = Vec2::new(
            self.width.unwrap_or_else(|| ui.available_width()),
            theme.row_height + HEADER_EXTRA,
        );
        let (rect, response) = match self.id {
            Some(id) => {
                let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
                (rect, ui.interact(rect, id, Sense::click()))
            }
            None => ui.allocate_exact_size(size, Sense::click()),
        };
        if !ui.is_rect_visible(rect) {
            return response;
        }
        let state = State::of(&response, self.focused);
        let painter = ui.painter();
        let code_font = theme.font(theme.size_code);
        let code_width = painter
            .layout_no_wrap(self.code.into(), code_font.clone(), theme.dim)
            .size()
            .x;
        let code_rect = Rect::from_min_size(
            rect.left_center() - Vec2::new(0.0, HEADER_CODE.y / 2.0),
            Vec2::new((code_width + HEADER_CODE.x).round(), HEADER_CODE.y),
        );
        let (code_fill, code_border, code_text, title_text) = match state {
            State::Rest => (theme.card, theme.text2, theme.text, theme.text),
            State::Hovered => (theme.card, theme.hot, theme.text, theme.title),
            State::Active => (theme.hot, theme.hot, theme.on_hot, theme.title),
        };
        painter.rect(
            code_rect,
            CornerRadius::same(MARKER_RADIUS),
            code_fill,
            Stroke::new(theme.stroke, code_border),
            StrokeKind::Inside,
        );
        painter.text(
            code_rect.center(),
            Align2::CENTER_CENTER,
            self.code,
            code_font,
            code_text,
        );
        let mut after = code_rect.right_center() + Vec2::new(HEADER_GAP, 0.0);
        if let Some((texture, size)) = self.mark {
            let scale = ui.ctx().pixels_per_point();
            let snap = |value: f32| (value * scale).round() / scale;
            let min = Pos2::new(snap(after.x), snap(rect.center().y - size.y / 2.0));
            let image = Rect::from_min_size(min, size);
            painter.image(
                texture,
                image,
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );
            after = Pos2::new(image.right() + HEADER_GAP, after.y);
        }
        let title = shaped(
            painter,
            self.title,
            theme.title_font(theme.size_small),
            title_text,
            theme.titles,
        );
        place(painter, after, Align2::LEFT_CENTER, title, title_text);
        if let Some(info) = self.info {
            let info = shaped(
                painter,
                info,
                theme.label_font(theme.size_code),
                theme.muted,
                theme.labels,
            );
            place(
                painter,
                rect.right_center(),
                Align2::RIGHT_CENTER,
                info,
                theme.muted,
            );
        }
        response
    }
}
