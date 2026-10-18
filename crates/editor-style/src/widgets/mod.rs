mod dotted;
mod header;
mod keycap;
mod number;
mod pill;
mod tabs;

pub use dotted::Dotted;
pub use header::Header;
pub use keycap::KeyCap;
pub use number::Number;
pub use pill::{Pill, Tone};
pub use tabs::Tabs;

use std::sync::Arc;

use egui::{
    Align2, Color32, CornerRadius, FontId, Frame, Galley, Margin, Painter, Pos2, Rect, Response,
    Stroke,
};

use crate::theme::{Caps, Theme};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Rest,
    Hovered,
    Active,
}

impl State {
    pub fn of(response: &Response, active: bool) -> State {
        if active || response.is_pointer_button_down_on() {
            State::Active
        } else if response.hovered() {
            State::Hovered
        } else {
            State::Rest
        }
    }

    pub fn border(self, theme: &Theme) -> Color32 {
        match self {
            State::Rest => theme.line,
            State::Hovered => theme.accent,
            State::Active => theme.hot,
        }
    }
}

pub fn card(theme: &Theme) -> Frame {
    Frame::NONE
        .fill(theme.card)
        .stroke(Stroke::new(theme.stroke, theme.card_line))
        .corner_radius(CornerRadius::same(theme.radius))
        .inner_margin(Margin::same(theme.panel_padding))
}

fn shaped(painter: &Painter, text: &str, font: FontId, colour: Color32, caps: Caps) -> Arc<Galley> {
    painter.layout_job(caps.job(text, font, colour))
}

fn place(
    painter: &Painter,
    at: Pos2,
    anchor: Align2,
    galley: Arc<Galley>,
    colour: Color32,
) -> Rect {
    let rect = anchor.anchor_size(at, galley.size());
    painter.galley(rect.min, galley, colour);
    rect
}
