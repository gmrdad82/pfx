use std::ops::RangeInclusive;

use egui::{CornerRadius, Pos2, Rect, Response, Sense, Stroke, Ui, Vec2, Widget};

use super::State;
use crate::theme::Theme;
use crate::tokens::{KNOB_RADIUS, KNOB_RING, RAIL};

pub struct Slider<'a> {
    value: &'a mut f32,
    range: RangeInclusive<f32>,
    width: Option<f32>,
}

impl<'a> Slider<'a> {
    pub fn new(value: &'a mut f32, range: RangeInclusive<f32>) -> Slider<'a> {
        Slider {
            value,
            range,
            width: None,
        }
    }

    pub fn width(mut self, width: f32) -> Slider<'a> {
        self.width = Some(width);
        self
    }
}

pub fn fraction(value: f32, range: &RangeInclusive<f32>) -> f32 {
    let span = range.end() - range.start();
    if span <= 0.0 {
        return 0.0;
    }
    ((value - range.start()) / span).clamp(0.0, 1.0)
}

impl Widget for Slider<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let theme = Theme::of(ui.ctx());
        let width = self.width.unwrap_or(ui.spacing().slider_width);
        let size = Vec2::new(width, ui.spacing().interact_size.y);
        let (rect, mut response) = ui.allocate_exact_size(size, Sense::click_and_drag());
        let left = rect.left() + KNOB_RADIUS;
        let right = (rect.right() - KNOB_RADIUS).max(left);
        if (response.dragged() || response.clicked())
            && let Some(pointer) = response.interact_pointer_pos()
        {
            let t = if right > left {
                ((pointer.x - left) / (right - left)).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let next = self.range.start() + t * (self.range.end() - self.range.start());
            if next != *self.value {
                *self.value = next;
                response.mark_changed();
            }
        }
        if !ui.is_rect_visible(rect) {
            return response;
        }
        let y = rect.center().y;
        let x = left + fraction(*self.value, &self.range) * (right - left);
        let enabled = ui.is_enabled();
        let state = State::of(&response, false);
        let (fill, knob, ring) = match state {
            _ if !enabled => (theme.dim, theme.dim, None),
            State::Rest => (theme.hot, theme.hot, None),
            State::Hovered => (theme.hot, theme.hot, Some(theme.accent)),
            State::Active => (theme.hot, theme.accent, Some(theme.hot)),
        };
        let painter = ui.painter();
        let rail = Rect::from_min_max(
            Pos2::new(left, y - RAIL / 2.0),
            Pos2::new(right, y + RAIL / 2.0),
        );
        let round = CornerRadius::same((RAIL / 2.0) as u8);
        painter.rect_filled(rail, round, theme.track);
        painter.rect_filled(
            Rect::from_min_max(rail.min, Pos2::new(x, rail.max.y)),
            round,
            fill,
        );
        painter.circle(
            Pos2::new(x, y),
            KNOB_RADIUS,
            knob,
            ring.map_or(Stroke::NONE, |colour| Stroke::new(KNOB_RING, colour)),
        );
        response
    }
}
