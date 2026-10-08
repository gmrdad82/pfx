use egui::{
    Align2, CornerRadius, Event, EventFilter, Key, Rect, Response, Sense, Stroke, StrokeKind, Ui,
    Vec2, Widget,
};

use super::{State, place, shaped};
use crate::theme::Theme;

pub struct Number<'a> {
    value: &'a mut f32,
    unit: &'a str,
    speed: f32,
    decimals: usize,
    width: f32,
}

impl<'a> Number<'a> {
    pub fn new(value: &'a mut f32) -> Number<'a> {
        Number {
            value,
            unit: "",
            speed: 0.01,
            decimals: 3,
            width: 76.0,
        }
    }

    pub fn unit(mut self, unit: &'a str) -> Number<'a> {
        self.unit = unit;
        self
    }

    pub fn speed(mut self, speed: f32) -> Number<'a> {
        self.speed = speed;
        self
    }

    pub fn decimals(mut self, decimals: usize) -> Number<'a> {
        self.decimals = decimals;
        self
    }

    pub fn width(mut self, width: f32) -> Number<'a> {
        self.width = width;
        self
    }
}

pub fn format(value: f32, decimals: usize) -> String {
    let text = format!("{value:.decimals$}");
    match text.strip_prefix('-') {
        Some(rest) if rest.chars().all(|c| c == '0' || c == '.') => rest.to_string(),
        _ => text,
    }
}

impl Widget for Number<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let theme = Theme::of(ui.ctx());
        let size = Vec2::new(self.width, theme.row_height);
        let (rect, mut response) = ui.allocate_exact_size(size, Sense::click_and_drag());
        let id = response.id;
        let mut editing: Option<String> = ui.data(|data| data.get_temp(id));
        if response.dragged() && editing.is_none() {
            let delta = response.drag_delta().x * self.speed;
            if delta != 0.0 {
                *self.value += delta;
                response.mark_changed();
            }
        }
        if response.clicked() && editing.is_none() {
            editing = Some(format(*self.value, self.decimals));
            response.request_focus();
        }
        if editing.is_some() && ui.memory(|memory| memory.has_focus(id)) {
            ui.memory_mut(|memory| {
                memory.set_focus_lock_filter(
                    id,
                    EventFilter {
                        tab: false,
                        horizontal_arrows: true,
                        vertical_arrows: true,
                        escape: true,
                    },
                )
            });
            let events = ui.input(|input| input.events.clone());
            for event in events {
                let Some(text) = editing.as_mut() else { break };
                match event {
                    Event::Text(typed) => text.push_str(&typed),
                    Event::Key {
                        key: Key::Backspace,
                        pressed: true,
                        ..
                    } => {
                        text.pop();
                    }
                    Event::Key {
                        key: Key::Enter,
                        pressed: true,
                        ..
                    } => {
                        if let Ok(value) = text.trim().parse::<f32>() {
                            *self.value = value;
                            response.mark_changed();
                        }
                        editing = None;
                        response.surrender_focus();
                    }
                    Event::Key {
                        key: Key::Escape,
                        pressed: true,
                        ..
                    } => {
                        editing = None;
                        response.surrender_focus();
                    }
                    _ => {}
                }
            }
        } else if let Some(text) = editing.take()
            && let Ok(value) = text.trim().parse::<f32>()
        {
            *self.value = value;
            response.mark_changed();
        }
        match &editing {
            Some(text) => ui.data_mut(|data| data.insert_temp(id, text.clone())),
            None => ui.data_mut(|data| data.remove::<String>(id)),
        }
        if !ui.is_rect_visible(rect) {
            return response;
        }
        let state = State::of(&response, editing.is_some() || response.dragged());
        let painter = ui.painter();
        painter.rect(
            rect,
            CornerRadius::same(theme.radius),
            theme.well,
            Stroke::new(theme.stroke, state.border(&theme)),
            StrokeKind::Inside,
        );
        let value_font = theme.font(theme.size_code);
        let unit_width = if self.unit.is_empty() {
            0.0
        } else {
            let unit = shaped(
                painter,
                self.unit,
                value_font.clone(),
                theme.muted,
                theme.labels,
            );
            place(
                painter,
                rect.right_center() - Vec2::new(6.0, 0.5),
                Align2::RIGHT_CENTER,
                unit,
                theme.muted,
            )
            .width()
                + 6.0
        };
        let right = rect.right_center() - Vec2::new(6.0 + unit_width, 0.5);
        match &editing {
            Some(text) => {
                let shown = painter.text(
                    right - Vec2::new(3.0, 0.0),
                    Align2::RIGHT_CENTER,
                    text,
                    value_font,
                    theme.text,
                );
                let caret = Rect::from_min_max(
                    shown.right_top() + Vec2::new(1.0, 0.0),
                    shown.right_bottom() + Vec2::new(3.0, 0.0),
                );
                painter.rect_filled(caret, CornerRadius::ZERO, theme.hot);
            }
            None => {
                painter.text(
                    right,
                    Align2::RIGHT_CENTER,
                    format(*self.value, self.decimals),
                    value_font,
                    theme.code,
                );
            }
        }
        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negative_zero_reads_as_zero() {
        assert_eq!(format(-0.0001, 3), "0.000");
        assert_eq!(format(-0.5, 2), "-0.50");
        assert_eq!(format(30.0, 0), "30");
    }
}
